

mod app;
mod error;
mod filesystem;
mod navigation;
mod operations;
mod ui;

use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox,
    Orientation, ScrolledWindow, SelectionMode,
};

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use app::state::AppState;
use filesystem::directory;
use operations::PendingOp;
use ui::dialogs;
use ui::file_view;

fn main() {
    let app = Application::builder()
        .application_id("org.mitos.file-manager")
        .build();

    app.connect_activate(build_ui);
    let _ = app.run();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("MITOS Files")
        .default_width(1024)
        .default_height(700)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 6);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let toolbar = GtkBox::new(Orientation::Horizontal, 6);

    let back_btn = Button::with_label("Back");
    let up_btn = Button::with_label("Up");
    let home_btn = Button::with_label("Home");
    let new_folder_btn = Button::with_label("New Folder");
    let new_file_btn = Button::with_label("New File");
    let rename_btn = Button::with_label("Rename");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let paste_btn = Button::with_label("Paste");
    let trash_btn = Button::with_label("Trash");
    let hidden_toggle = CheckButton::with_label("Hidden");

    toolbar.append(&back_btn);
    toolbar.append(&up_btn);
    toolbar.append(&home_btn);
    toolbar.append(&new_folder_btn);
    toolbar.append(&new_file_btn);
    toolbar.append(&rename_btn);
    toolbar.append(&copy_btn);
    toolbar.append(&move_btn);
    toolbar.append(&paste_btn);
    toolbar.append(&trash_btn);
    toolbar.append(&hidden_toggle);

    let location = Entry::new();
    location.set_placeholder_text(Some("/path/to/directory"));
    location.set_hexpand(true);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    let status = Label::new(Some("Ready"));
    status.set_halign(gtk::Align::Start);

    root.append(&toolbar);
    root.append(&location);
    root.append(&scrolled);
    root.append(&status);

    window.set_child(Some(&root));

    let state = Rc::new(RefCell::new(AppState::new(home_dir())));

    refresh(&state, &list, &location, &status);

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        back_btn.connect_clicked(move |_| {
            go_back(&state, &list, &location, &status);
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        up_btn.connect_clicked(move |_| {
            let current = state.borrow().current.clone();

            if let Some(parent) = current.parent() {
                navigate_to(&state, &list, &location, &status, parent.to_path_buf());
            } else {
                status.set_label("Already at the top level.");
            }
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        home_btn.connect_clicked(move |_| {
            navigate_to(&state, &list, &location, &status, home_dir());
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let status = status.clone();

        location.connect_activate(move |entry| {
            let text = entry.text().to_string();
            let path = PathBuf::from(text);

            if path.is_dir() {
                navigate_to(&state, &list, entry, &status, path);
            } else {
                status.set_label("That location is not a directory.");
            }
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        hidden_toggle.connect_toggled(move |toggle| {
            state.borrow_mut().show_hidden = toggle.is_active();
            refresh(&state, &list, &location, &status);
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        list.connect_row_activated(move |_, row| {
            let index = row.index();

            if index < 0 {
                return;
            }

            let item = state.borrow().items.get(index as usize).cloned();

            if let Some(item) = item {
                if item.is_dir {
                    navigate_to(&state, &list, &location, &status, item.path);
                } else {
                    let _ = Command::new("xdg-open").arg(&item.path).spawn();
                }
            }
        });
    }

    {
        let window_parent = window.clone();
        let window_error = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        new_folder_btn.connect_clicked(move |_| {
            dialogs::show_text_dialog(
                &window_parent,
                "New Folder",
                "New Folder",
                "Create",
                {
                    let window_error = window_error.clone();
                    let state = state.clone();
                    let list = list.clone();
                    let location = location.clone();
                    let status = status.clone();

                    move |name| {
                        if name.is_empty() {
                            return;
                        }

                        let parent = state.borrow().current.clone();

                        if let Err(err) = operations::create::create_folder(&parent, &name) {
                            dialogs::show_error(
                                &window_error,
                                &format!("Could not create folder: {err}"),
                            );
                        }

                        refresh(&state, &list, &location, &status);
                    }
                },
            );
        });
    }

    {
        let window_parent = window.clone();
        let window_error = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        new_file_btn.connect_clicked(move |_| {
            dialogs::show_text_dialog(
                &window_parent,
                "New File",
                "new-file.txt",
                "Create",
                {
                    let window_error = window_error.clone();
                    let state = state.clone();
                    let list = list.clone();
                    let location = location.clone();
                    let status = status.clone();

                    move |name| {
                        if name.is_empty() {
                            return;
                        }

                        let parent = state.borrow().current.clone();

                        if let Err(err) = operations::create::create_file(&parent, &name) {
                            dialogs::show_error(
                                &window_error,
                                &format!("Could not create file: {err}"),
                            );
                        }

                        refresh(&state, &list, &location, &status);
                    }
                },
            );
        });
    }

    {
        let window_parent = window.clone();
        let window_error = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        rename_btn.connect_clicked(move |_| {
            let selected = {
                let s = state.borrow();
                file_view::selected_item(&list, &s.items)
            };

            let Some(item) = selected else {
                status.set_label("Select a file or folder to rename.");
                return;
            };

            let source = item.path.clone();
            let initial_name = item.name.clone();

            dialogs::show_text_dialog(&window_parent, "Rename", &initial_name, "Rename", {
                let window_error = window_error.clone();
                let state = state.clone();
                let list = list.clone();
                let location = location.clone();
                let status = status.clone();

                move |name| {
                    if name.is_empty() {
                        return;
                    }

                    if let Err(err) = operations::rename::rename_path(&source, &name) {
                        dialogs::show_error(&window_error, &format!("Could not rename: {err}"));
                    }

                    refresh(&state, &list, &location, &status);
                }
            });
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let status = status.clone();

        copy_btn.connect_clicked(move |_| {
            let selected = {
                let s = state.borrow();
                file_view::selected_item(&list, &s.items)
            };

            if let Some(item) = selected {
                state.borrow_mut().pending = Some((PendingOp::Copy, item.path.clone()));
                status.set_label(&format!(
                    "Copied “{}”. Navigate to destination and press Paste.",
                    item.name
                ));
            } else {
                status.set_label("Select a file or folder to copy.");
            }
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let status = status.clone();

        move_btn.connect_clicked(move |_| {
            let selected = {
                let s = state.borrow();
                file_view::selected_item(&list, &s.items)
            };

            if let Some(item) = selected {
                state.borrow_mut().pending = Some((PendingOp::Move, item.path.clone()));
                status.set_label(&format!(
                    "Marked “{}” for move. Navigate to destination and press Paste.",
                    item.name
                ));
            } else {
                status.set_label("Select a file or folder to move.");
            }
        });
    }

    {
        let window_error = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        paste_btn.connect_clicked(move |_| {
            let pending = state.borrow_mut().pending.take();

            let Some((operation, source)) = pending else {
                status.set_label("Nothing to paste. Copy or move an item first.");
                return;
            };

            let destination_dir = state.borrow().current.clone();
            let file_name = source.file_name().unwrap_or_default().to_os_string();
            let destination = operations::unique_destination(&destination_dir.join(file_name));

            let result = match operation {
                PendingOp::Copy => operations::copy::copy_path(&source, &destination),
                PendingOp::Move => operations::move_op::move_path(&source, &destination),
            };

            if let Err(err) = result {
                dialogs::show_error(&window_error, &format!("Paste failed: {err}"));
            }

            refresh(&state, &list, &location, &status);
        });
    }

    {
        let window_error = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        trash_btn.connect_clicked(move |_| {
            let selected = {
                let s = state.borrow();
                file_view::selected_item(&list, &s.items)
            };

            if let Some(item) = selected {
                match operations::trash::delete(&item.path) {
                    Ok(_) => {
                        status.set_label(&format!("Moved “{}” to trash.", item.name));
                    }
                    Err(err) => {
                        dialogs::show_error(
                            &window_error,
                            &format!("Could not move to trash: {err}"),
                        );
                    }
                }

                refresh(&state, &list, &location, &status);
            } else {
                status.set_label("Select a file or folder to move to trash.");
            }
        });
    }

    window.present();
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn normalize(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn navigate_to(
    state: &Rc<RefCell<AppState>>,
    list: &ListBox,
    location: &Entry,
    status: &Label,
    requested: PathBuf,
) {
    let path = normalize(requested);

    if !path.is_dir() {
        status.set_label("That location is not a directory.");
        return;
    }

    {
        let mut s = state.borrow_mut();

        if s.current != path {
            s.history.push(s.current.clone());
            s.current = path;
        }
    }

    refresh(state, list, location, status);
}

fn go_back(state: &Rc<RefCell<AppState>>, list: &ListBox, location: &Entry, status: &Label) {
    let previous = state.borrow_mut().history.pop();

    if let Some(previous) = previous {
        state.borrow_mut().current = previous;
        refresh(state, list, location, status);
    } else {
        status.set_label("No previous location.");
    }
}

fn refresh(state: &Rc<RefCell<AppState>>, list: &ListBox, location: &Entry, status: &Label) {
    let mut s = state.borrow_mut();
    let current = s.current.clone();

    location.set_text(&current.display().to_string());

    file_view::clear(list);

    let items = directory::read_items(&current, s.show_hidden);
    file_view::render(list, &items);

    status.set_label(&format!("{} · {} items", current.display(), items.len()));

    s.items = items;
}
