#![allow(dead_code)]

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
use filesystem::metadata;
use navigation::locations;
use operations::PendingOp;
use ui::dialogs;
use ui::file_view;
use ui::sidebar;

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
        .default_width(1100)
        .default_height(720)
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

    let sidebar_list = ListBox::new();
    sidebar_list.set_selection_mode(SelectionMode::Single);
    sidebar::build(&sidebar_list);

    let sidebar_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    sidebar_scrolled.set_child(Some(&sidebar_list));
    sidebar_scrolled.set_width_request(190);
    sidebar_scrolled.set_vexpand(true);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Multiple);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    let content = GtkBox::new(Orientation::Horizontal, 6);
    content.append(&sidebar_scrolled);
    content.append(&scrolled);
    content.set_vexpand(true);

    let status = Label::new(Some("Ready"));
    status.set_halign(gtk::Align::Start);

    root.append(&toolbar);
    root.append(&location);
    root.append(&content);
    root.append(&status);

    window.set_child(Some(&root));

    let state = Rc::new(RefCell::new(AppState::new(locations::home_dir())));

    refresh(&state, &list, &location, &status);

    {
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        sidebar_list.connect_row_activated(move |_, row| {
            let index = row.index();

            if index < 0 {
                return;
            }

            if let Some(path) = sidebar::place_at(index as usize) {
                navigate_to(&state, &list, &location, &status, path);
            }
        });
    }

    {
        let window = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            if let Some(row) = list.row_at_y(y as i32) {
                let index = row.index();

                if index >= 0 {
                    if !row.is_selected() {
                        list.unselect_all();
                        list.select_row(&row);
                    }

                    let items = {
                        let s = state.borrow();
                        file_view::selected_items(&list, &s.items)
                    };

                    if !items.is_empty() {
                        show_context_menu(&window, &state, &list, &location, &status, items, x, y);
                    }
                }
            }
        });

        list.add_controller(right_click);
    }

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
            navigate_to(&state, &list, &location, &status, locations::home_dir());
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
                file_view::selected_items(&list, &s.items)
            };

            if selected.len() != 1 {
                status.set_label("Select exactly one file or folder to rename.");
                return;
            }

            let item = selected[0].clone();
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
                file_view::selected_items(&list, &s.items)
            };

            if selected.is_empty() {
                status.set_label("Select one or more files or folders to copy.");
                return;
            }

            let paths: Vec<PathBuf> = selected.iter().map(|item| item.path.clone()).collect();
            let count = paths.len();

            state.borrow_mut().pending = Some((PendingOp::Copy, paths));

            status.set_label(&format!(
                "Copied {count} item(s). Navigate to destination and press Paste."
            ));
        });
    }

    {
        let state = state.clone();
        let list = list.clone();
        let status = status.clone();

        move_btn.connect_clicked(move |_| {
            let selected = {
                let s = state.borrow();
                file_view::selected_items(&list, &s.items)
            };

            if selected.is_empty() {
                status.set_label("Select one or more files or folders to move.");
                return;
            }

            let paths: Vec<PathBuf> = selected.iter().map(|item| item.path.clone()).collect();
            let count = paths.len();

            state.borrow_mut().pending = Some((PendingOp::Move, paths));

            status.set_label(&format!(
                "Marked {count} item(s) for move. Navigate to destination and press Paste."
            ));
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

            let Some((operation, sources)) = pending else {
                status.set_label("Nothing to paste. Copy or move items first.");
                return;
            };

            let destination_dir = state.borrow().current.clone();

            match operations::paste_pending(&destination_dir, operation, &sources) {
                Ok(count) => {
                    status.set_label(&format!("Pasted {count} item(s)."));
                }
                Err(err) => {
                    dialogs::show_error(&window_error, &format!("Paste failed: {err}"));
                }
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
                file_view::selected_items(&list, &s.items)
            };

            if selected.is_empty() {
                status.set_label("Select one or more files or folders to move to trash.");
                return;
            }

            let mut succeeded = 0;
            let mut failed = 0;
            let mut last_error = None;

            for item in &selected {
                match operations::trash::delete(&item.path) {
                    Ok(_) => {
                        succeeded += 1;
                    }
                    Err(err) => {
                        failed += 1;
                        last_error = Some(err);
                    }
                }
            }

            if failed == 0 {
                status.set_label(&format!("Moved {succeeded} item(s) to trash."));
            } else if succeeded == 0 {
                if let Some(err) = last_error {
                    dialogs::show_error(&window_error, &format!("Could not move to trash: {err}"));
                }
            } else {
                status.set_label(&format!(
                    "Moved {succeeded} item(s) to trash. {failed} failed."
                ));
            }

            refresh(&state, &list, &location, &status);
        });
    }

    window.present();
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

fn show_context_menu(
    window: &ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    list: &ListBox,
    location: &Entry,
    status: &Label,
    items: Vec<directory::Item>,
    x: f64,
    y: f64,
) {
    if items.is_empty() {
        return;
    }

    let count = items.len();
    let single_item = items.first().cloned();

    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(list);
    popover.set_pointing_to(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);

    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let open_btn = Button::with_label("Open");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let rename_btn = Button::with_label("Rename");
    let trash_btn = Button::with_label("Trash");
    let properties_btn = Button::with_label("Properties");

    open_btn.set_sensitive(count == 1);
    rename_btn.set_sensitive(count == 1);
    properties_btn.set_sensitive(count == 1);

    menu_box.append(&open_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&move_btn);
    menu_box.append(&rename_btn);
    menu_box.append(&trash_btn);
    menu_box.append(&properties_btn);

    popover.set_child(Some(&menu_box));

    {
        let popover = popover.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();
        let item = single_item.clone();

        open_btn.connect_clicked(move |_| {
            popover.popdown();

            let Some(item) = item.clone() else {
                return;
            };

            if item.is_dir {
                navigate_to(&state, &list, &location, &status, item.path.clone());
            } else {
                let _ = Command::new("xdg-open").arg(&item.path).spawn();
            }
        });
    }

    {
        let popover = popover.clone();
        let state = state.clone();
        let status = status.clone();

        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
        let count = paths.len();

        copy_btn.connect_clicked(move |_| {
            popover.popdown();

            state.borrow_mut().pending = Some((PendingOp::Copy, paths.clone()));

            status.set_label(&format!(
                "Copied {count} item(s). Navigate to destination and press Paste."
            ));
        });
    }

    {
        let popover = popover.clone();
        let state = state.clone();
        let status = status.clone();

        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
        let count = paths.len();

        move_btn.connect_clicked(move |_| {
            popover.popdown();

            state.borrow_mut().pending = Some((PendingOp::Move, paths.clone()));

            status.set_label(&format!(
                "Marked {count} item(s) for move. Navigate to destination and press Paste."
            ));
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();
        let single_item = single_item.clone();

        rename_btn.connect_clicked(move |_| {
            popover.popdown();

            let Some(item) = single_item.clone() else {
                return;
            };

            let source = item.path.clone();
            let initial_name = item.name.clone();

            dialogs::show_text_dialog(&window, "Rename", &initial_name, "Rename", {
                let window = window.clone();
                let state = state.clone();
                let list = list.clone();
                let location = location.clone();
                let status = status.clone();

                move |name| {
                    if name.is_empty() {
                        return;
                    }

                    if let Err(err) = operations::rename::rename_path(&source, &name) {
                        dialogs::show_error(&window, &format!("Could not rename: {err}"));
                    }

                    refresh(&state, &list, &location, &status);
                }
            });
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let state = state.clone();
        let list = list.clone();
        let location = location.clone();
        let status = status.clone();

        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
        let count = paths.len();

        trash_btn.connect_clicked(move |_| {
            popover.popdown();

            let mut succeeded = 0;
            let mut failed = 0;
            let mut last_error = None;

            for path in &paths {
                match operations::trash::delete(path) {
                    Ok(_) => {
                        succeeded += 1;
                    }
                    Err(err) => {
                        failed += 1;
                        last_error = Some(err);
                    }
                }
            }

            if failed == 0 {
                status.set_label(&format!("Moved {succeeded} item(s) to trash."));
            } else if succeeded == 0 {
                if let Some(err) = last_error {
                    dialogs::show_error(&window, &format!("Could not move to trash: {err}"));
                }
            } else {
                status.set_label(&format!(
                    "Moved {succeeded} item(s) to trash. {failed} of {count} failed."
                ));
            }

            refresh(&state, &list, &location, &status);
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let item = single_item.clone();

        properties_btn.connect_clicked(move |_| {
            popover.popdown();

            let Some(item) = item.clone() else {
                return;
            };

            let size = if item.is_dir {
                "-".to_string()
            } else {
                metadata::format_size(item.metadata.size)
            };

            let modified = metadata::format_modified(item.metadata.modified);
            let permissions = item.metadata.permissions.clone();
            let is_symlink = item.metadata.is_symlink;

            let kind = if item.is_dir { "Folder" } else { "File" };

            let message = format!(
                "Name: {}\nPath: {}\nType: {}\nSize: {}\nModified: {}\nPermissions: {}\nSymlink: {}",
                item.name,
                item.path.display(),
                kind,
                size,
                modified,
                permissions,
                is_symlink
            );

            dialogs::show_info(&window, "Properties", &message);
        });
    }

    popover.popup();
}
