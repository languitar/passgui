#[macro_use]
extern crate log;
extern crate log4rs;
extern crate gdk;
extern crate glib;
extern crate gtk;
extern crate walkdir;

use std::cell::RefCell;
use std::env;
use std::ffi;
use std::str;
use std::thread;

use std::process::Command;
use std::string::String;
use std::vec::Vec;

use walkdir::{DirEntry, WalkDir, WalkDirIterator};

use gtk::prelude::*;
use gtk::{ButtonsType, DialogFlags, Entry, EntryCompletion, ListStore, MessageType,
          MessageDialog, Type, Window, WindowPosition, WindowType};

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
        let relative = path.strip_prefix(&store_dir)
                           .expect("A contained file in the password store must always \
                                    be relative to it");
        // strip the gpg extension
        // first, get the folder containg the gpg file or use an empty folder if it is in the base
        let folder = relative.parent().unwrap_or(std::path::Path::new(""));
        // then join the filename without the eztension to this folder to compose the common name
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

fn get_password(entry: &String) -> Result<String, String> {
    debug!("Trying to get password for entry '{:?}'", entry);
    let output = try!(Command::new("pass")
                              .arg("show")
                              .arg(entry)
                              .output()
                              .map_err(|e| format!("Unable to call pass: {:?}", e.to_string())));
    if !output.status.success() {
        return Err(format!("pass indicated an error:\n stdout:\n{:?}\n\nstderr:\n{:?}",
                           std::str::from_utf8(&output.stdout),
                           std::str::from_utf8(&output.stderr)));
    }

    let out = try!(std::str::from_utf8(&output.stdout)
                            .map_err(|e| format!("Unable to decode reply from pass: {:?}",
                                                 e.to_string()))
                            .map(|s| s.to_string()));
    let password = try!(out.lines().next().ok_or("pass output did not contain any lines")
                                          .map(|l| l.to_string()));
    debug!("Received password of length {:?}", password.len());

    Ok(password)
}

fn auto_type(text: &String) -> Result<(), String> {
    debug!("Auto-typing text of length {:?}",
           text.len());
    let status = try!(Command::new("cliclick")
                              .arg("t:".to_string() + text)
                              .status()
                              .map_err(|e| format!("Could not launch clicick: {:?}:",
                                                   e.to_string())));
    if status.success() {
        Ok(())
    } else {
        Err("clicick was not successful".to_string())
    }
}

fn get_previous_app() -> Result<String, String> {
    debug!("Receiving previously focused app from system");
    let output = try!(Command::new("osascript")
                              .arg("-e")
                              .arg("tell application \"System Events\" \
                                    to set frontmostApplicationName \
                                    to name of 1st process whose frontmost is true")
                              .output()
                              .map_err(|e| format!("Could not call osascript: {:?}",
                                                   e.to_string())));
    match std::str::from_utf8(&output.stdout) {
        Ok(s)  => Ok(s.to_string().trim().to_string()),
        Err(_) => Err("UTF-8 conversion error for entry".to_string())
    }
}

fn focus_app(name: &String) -> Result<(), String> {
    debug!("Focusing app {:?}", name);
    let status = try!(Command::new("osascript")
                              .arg("-e")
                              .arg(format!("tell application \"{}\" to activate", name))
                              .status()
                              .map_err(|e| format!("Could not call osascript: {:?}",
                                                   e.to_string())));
    if status.success() {
        Ok(())
    } else {
        Err("osascript was not successful".to_string())
    }
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
    let previous_app = get_previous_app().expect("Unable to function without previous app");
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
    // hack around the nasty ownership issues with global state
    search_entry.connect_activate(|_| {
        GLOBAL.with(|global| {
            let borrowed = global.borrow();
            let (previous_app, choices, entry) = match *borrowed {
                Some((ref previous_app, ref choices, _, ref entry)) => (
                    previous_app, choices, entry),
                None => panic!(),
            };

            let current_text = entry.get_text().unwrap_or(String::new());
            debug!("Form activated with text {:?}", current_text);

            if !choices.contains(&current_text) {
                return;
            }

            debug!("Entered text is a valid password store entry, continuing");

            entry.set_sensitive(false);

            // request password
            let next_app = previous_app.clone();
            thread::spawn(move || {
                let password = match get_password(&current_text) {
                    Ok(p) => p,
                    Err(error) =>{
                        signal_error(error);
                        return
                    },
                };

                match focus_app(&next_app) {
                    Ok(_) => {},
                    Err(error) => {
                        signal_error(error);
                        return
                    },
                }

                // auto type
                match auto_type(&password) {
                    Ok(_) => {},
                    Err(error) => {
                        signal_error(error);
                        return
                    },
                }

                glib::idle_add(exit);
            });

        });
    });

    GLOBAL.with(move |global| {
        *global.borrow_mut() = Some((previous_app, choices, window, search_entry))
    });

    gtk::main();
}

// declare a new thread local storage key
thread_local!(
    static GLOBAL: RefCell<Option<(String, Vec<String>, gtk::Window, gtk::Entry)>> = RefCell::new(None)
);

fn signal_error(message: String) {
    glib::idle_add(move || {
        let dialog = MessageDialog::new(None::<&Window>,
                                        DialogFlags::empty(),
                                        MessageType::Error,
                                        ButtonsType::Ok,
                                        format!("{}{}", "Error:\n\n", message).as_str());
        dialog.set_position(WindowPosition::Center);
        dialog.run();
        dialog.destroy();
        GLOBAL.with(|global| {
            let borrowed = global.borrow();
            let entry = match *borrowed {
                Some((_, _, _, ref entry)) => entry,
                None => panic!(),
            };
            entry.set_sensitive(true);
        });
        glib::Continue(false)
    });
}

fn exit() -> glib::Continue {
    gtk::main_quit();
    glib::Continue(false)
}
