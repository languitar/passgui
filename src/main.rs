#[macro_use]
extern crate log;
extern crate log4rs;
extern crate gdk;
extern crate gtk;
extern crate walkdir;

use std::env;
use std::ffi;
use std::str;

use std::process::Command;
use std::string::String;
use std::vec::Vec;

use walkdir::{DirEntry, WalkDir, WalkDirIterator};

use gtk::prelude::*;
use gtk::{Entry, EntryCompletion, ListStore, Type, Window, WindowPosition, WindowType};

use log::LogLevelFilter;
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Config, Root};

fn is_hidden(entry: &DirEntry) -> bool {
    entry.file_name()
         .to_str()
         .map(|s| s.starts_with("."))
         .unwrap_or(false)
}

fn is_gpg_file(entry: &DirEntry) -> bool {
    entry.file_type().is_file() &&
        entry.path().extension().unwrap_or(ffi::OsStr::new("")) == "gpg"
}

fn get_choices() -> Vec<String> {
    // TODO custom password store via ENV var
    let home = env::home_dir().unwrap();
    let store_dir = home.join(".password-store");

    debug!("Collecting password store entries from folder {:?}", store_dir.to_str());

    let mut choices: Vec<String> = Vec::new();

    let walker = WalkDir::new(&store_dir).min_depth(1).into_iter();
    for entry in walker.filter_entry(|e| !is_hidden(e)) {
        let entry = entry.unwrap();
        trace!("Probing entry {:?}", entry);

        // skip anything that is not a gpg file
        // can't be done with filter_entry as this would then skip all directories
        if !is_gpg_file(&entry) {
            trace!("Skipping entry {:?} as this is no GPG file", entry);
            continue;
        }

        let path = entry.path();

        // get the entry relative to the store root
        let relative = path.strip_prefix(&store_dir).unwrap();
        // strip the gpg extension
        let folder = relative.parent().unwrap_or(std::path::Path::new(""));
        let name = folder.join(relative.file_stem().expect(
                "Entry must have a stem because we only allow files with extension here"));

        choices.push(name.to_str().unwrap().to_string())
    }

    trace!("Final list of names: {:?}", choices);

    choices
}

fn choices_to_model(choices: &Vec<String>) -> ListStore {
    debug!("Converting {:?} choices to Gtk ListStore", choices.len());
    let model = ListStore::new(&[Type::String]);
    for name in choices {
        model.insert_with_values(None, &[0], &[&name]);
    }
    model
}

fn get_password(entry: &String) -> String {
    debug!("Trying to get password for entry '{:?}'", entry);
    let output = Command::new("pass")
                         .arg("show")
                         .arg(entry)
                         .output()
                         .expect("Unable to get the password from pass");
    if !output.status.success() {
        // TODO check that this does not leak passwords to log files
        error!("Could not get the password:\n stdout:\n{:?}\n\nstderr:\n{:?}",
               std::str::from_utf8(&output.stdout),
               std::str::from_utf8(&output.stderr));
        panic!("pass did not return the password");
    }

    let out = std::str::from_utf8(&output.stdout)
                       .expect("UTF-8 conversion error for entry").to_string();
    let password = out.lines().next().expect("pass entry did not contain any lines").to_string();
    debug!("Received password of length {:?}", password.len());

    password
}

fn auto_type(text: &String) {
    debug!("Auto-typeing text of length {:?}",
           text.len());
    Command::new("cliclick")
            .arg("t:".to_string() + text)
            .status().expect("Could not type");
}

fn get_previous_app() -> String {
    debug!("Receiving previously focused app from system");
    let output = Command::new("osascript")
                         .arg("-e")
                         .arg("tell application \"System Events\" to set frontmostApplicationName to name of 1st process whose frontmost is true")
                         .output()
                         .expect("Could not get previous app");
    std::str::from_utf8(&output.stdout)
             .expect("UTF-8 conversion error for entry").to_string().trim().to_string()
}

fn focus_app(name: &String) {
    debug!("Focusing app {:?}", name);
    Command::new("osascript")
            .arg("-e")
            .arg(format!("tell application \"{}\" to activate", name))
            .status().expect("Could not activate recent app");
}

fn configure_logging() {
    let stdout = ConsoleAppender::builder().build();
    let config = Config::builder()
                        .appender(Appender::builder().build("stdout", Box::new(stdout)))
                        .build(Root::builder().appender("stdout").build(LogLevelFilter::Trace))
                        .unwrap();
    log4rs::init_config(config).expect("Unable to initialize logging");
}

fn main() {
    configure_logging();

    // get the currently active app to restore focus in case we want to type
    let previous_app = get_previous_app();
    debug!("Determied previous app to be {:?}", previous_app);

    gtk::init().expect("Failed to initialize GTK");
    trace!("Initializing GTK succeeded");

    let choices = get_choices();

    let window = Window::new(WindowType::Toplevel);
    window.set_title("passgui");
    window.set_position(WindowPosition::Center);
    window.set_size_request(300, 10);
    window.set_modal(true);
    window.set_keep_above(true);

    let completion = EntryCompletion::new();
    let model = choices_to_model(&choices);
    completion.set_model(Some(&model));
    completion.set_text_column(0);
    completion.set_inline_completion(true);
    completion.set_popup_completion(true);
    completion.set_popup_single_match(true);
    // TODO use substring match one supported by gtk-rs

    let search_entry = Entry::new();
    search_entry.set_completion(Some(&completion));
    window.add(&search_entry);

    window.show_all();

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    // quit on escape
    search_entry.connect_key_release_event(|_, key| {
        trace!("Received key_release_event {:?} with keyval {}", key, key.get_keyval());
        if key.get_keyval() == gdk::enums::key::Escape {
            info!("Escape pressed, exiting");
            gtk::main_quit();
        }
        Inhibit(false)
    });

    // Execute on enter
    // hack around the nasty ownership issues here by moving the search entry into the clouse.
    search_entry.connect_activate(move |entry| {
        let current_text = entry.get_text().unwrap_or(String::new());
        debug!("Form activated with text {:?}", current_text);

        if choices.contains(&current_text) {
            debug!("Entered text is a valid password store entry, continuing");

            window.hide();

            // request password
            let password = get_password(&current_text);

            // restore focus for previous app
            focus_app(&previous_app);

            // auto type
            auto_type(&password);

            // end this process
            gtk::main_quit();
        }
    });

    gtk::main();
}
