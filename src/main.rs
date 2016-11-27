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

    let mut choices: Vec<String> = Vec::new();

    let walker = WalkDir::new(&store_dir).min_depth(1).into_iter();
    for entry in walker.filter_entry(|e| !is_hidden(e)) {
        let entry = entry.unwrap();

        // skip anything that is not a gpg file
        // can't be done with filter_entry as this would then skip all directories
        if !is_gpg_file(&entry) {
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

    choices
}

fn choices_to_model(choices: &Vec<String>) -> ListStore {
    let model = ListStore::new(&[Type::String]);
    for name in choices {
        model.insert_with_values(None, &[0], &[&name]);
    }
    model
}

fn get_password(entry: &String) -> String {
    let output = Command::new("pass")
                         .arg("show")
                         .arg(entry)
                         .output()
                         .expect("Unable to get the password from pass");
    if !output.status.success() {
        panic!("pass did not return the password");
    }

    let out = std::str::from_utf8(&output.stdout)
                       .expect("UTF-8 conversion error for entry").to_string();

    out.lines().next().expect("pass entry did not contain any lines").to_string()
}

fn auto_type(text: &String) {
    Command::new("cliclick")
            .arg("t:".to_string() + text)
            .status().expect("Could not type");
}

fn get_previous_app() -> String {
    let output = Command::new("osascript")
                         .arg("-e")
                         .arg("tell application \"System Events\" to set frontmostApplicationName to name of 1st process whose frontmost is true")
                         .output()
                         .expect("Could not get previous app");
    std::str::from_utf8(&output.stdout)
             .expect("UTF-8 conversion error for entry").to_string().trim().to_string()
}

fn focus_app(name: &String) {
    Command::new("osascript")
            .arg("-e")
            .arg(format!("tell application \"{}\" to activate", name))
            .status().expect("Could not activate recent app");
}

fn main() {
    // get the currently active app to restore focus in case we want to type
    let previous_app = get_previous_app();
    println!("Previous: {}", previous_app);

    gtk::init().expect("Failed to initialize GTK");

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
        if key.get_keyval() == gdk::enums::key::Escape {
            gtk::main_quit();
        }
        Inhibit(false)
    });

    // Execute on enter
    // hack around the nasty ownership issues here by moving the search entry into the clouse.
    search_entry.connect_activate(move |entry| {
        let current_text = entry.get_text().unwrap_or(String::new());

        if choices.contains(&current_text) {
            println!("Here we GO!");

            window.hide();

            // reuest password
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
