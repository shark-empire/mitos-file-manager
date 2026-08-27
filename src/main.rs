use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Dialog, Entry, Label,
    ListBox, ListBoxRow, Orientation, ResponseType, ScrolledWindow, SelectionMode,
};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

#[derive(Clone, Copy)]
enum PendingOp {
    Copy,
    Move,
}

#[derive(Clone)]
struct Item {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

struct State {
    current: PathBuf,
    history: Vec<PathBuf>,
    pending: Option<(PendingOp, PathBuf)>,
    show_hidden: bool,
    items: Vec<Item>,
}

impl State {
    fn new() -> Self {
        Self {
            current: home_dir(),
            history: Vec::new(),
            pending: None,
            show_hidden: false,
            items: Vec::new(),
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

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

    let state = Rc::new(RefCell::new(State::new()));

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
            show_text_dialog(
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

                        let target = state.borrow().current.join(&name);

                        if let Err(err) = fs::create_dir_all(&target) {
                            show_error(&window_error, &format!("Could not create folder: {err}"));
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
            show_text_dialog(
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

                        let target = state.borrow().current.join(&name);

                        if let Err(err) = fs::File::create(&target) {
                            show_error(&window_error, &format!("Could not create file: {err}"));
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
            let Some(item) = selected_item(&state, &list) else {
                status.set_label("Select a file or folder to rename.");
                return;
            };

            let source = item.path.clone();
            let initial_name = item.name.clone();

            show_text_dialog(&window_parent, "Rename", &initial_name, "Rename", {
                let window_error = window_error.clone();
                let state = state.clone();
                let list = list.clone();
                let location = location.clone();
                let status = status.clone();

                move |name| {
                    if name.is_empty() {
                        return;
                    }

                    let destination = source.with_file_name(&name);

                    if let Err(err) = fs::rename(&source, &destination) {
                        show_error(&window_error, &format!("Could not rename: {err}"));
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
            if let Some(item) = selected_item(&state, &list) {
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
            if let Some(item) = selected_item(&state, &list) {
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
            let destination = unique_destination(&destination_dir.join(file_name));

            let result = match operation {
                PendingOp::Copy => copy_path(&source, &destination),
                PendingOp::Move => move_path(&source, &destination),
            };

            if let Err(err) = result {
                show_error(&window_error, &format!("Paste failed: {err}"));
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
            if let Some(item) = selected_item(&state, &list) {
                match trash::delete(&item.path) {
                    Ok(_) => {
                        status.set_label(&format!("Moved “{}” to trash.", item.name));
                    }
                    Err(err) => {
                        show_error(&window_error, &format!("Could not move to trash: {err}"));
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

fn selected_item(state: &Rc<RefCell<State>>, list: &ListBox) -> Option<Item> {
    let row = list.selected_row()?;
    let index = row.index();

    if index < 0 {
        return None;
    }

    let state = state.borrow();
    state.items.get(index as usize).cloned()
}

fn normalize(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn navigate_to(
    state: &Rc<RefCell<State>>,
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

fn go_back(state: &Rc<RefCell<State>>, list: &ListBox, location: &Entry, status: &Label) {
    let previous = state.borrow_mut().history.pop();

    if let Some(previous) = previous {
        state.borrow_mut().current = previous;
        refresh(state, list, location, status);
    } else {
        status.set_label("No previous location.");
    }
}

fn refresh(state: &Rc<RefCell<State>>, list: &ListBox, location: &Entry, status: &Label) {
    let mut s = state.borrow_mut();
    let current = s.current.clone();

    location.set_text(&current.display().to_string());

    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let mut items = Vec::new();

    if let Ok(read_dir) = fs::read_dir(&current) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            if !s.show_hidden && name.starts_with('.') {
                continue;
            }

            let path = entry.path();
            let is_dir = path.is_dir();

            items.push(Item { path, name, is_dir });
        }
    }

    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    for item in &items {
        let row = ListBoxRow::new();

        let kind = if item.is_dir { "[dir]" } else { "[file]" };

        let label = Label::new(Some(&format!("{kind} {}", item.name)));
        label.set_halign(gtk::Align::Start);

        row.set_child(Some(&label));
        list.append(&row);
    }

    status.set_label(&format!("{} · {} items", current.display(), items.len()));

    s.items = items;
}

fn show_text_dialog<F>(
    parent: &ApplicationWindow,
    title: &str,
    initial: &str,
    ok_label: &str,
    on_accept: F,
) where
    F: Fn(String) + 'static,
{
    let dialog = Dialog::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button(ok_label, ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let entry = Entry::new();
    entry.set_text(initial);

    content.append(&entry);

    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let text = entry.text().to_string();
            on_accept(text.trim().to_string());
        }

        dialog.close();
    });

    dialog.present();
}

fn show_error(parent: &ApplicationWindow, message: &str) {
    let dialog = Dialog::builder()
        .title("Error")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("OK", ResponseType::Close);

    let label = Label::new(Some(message));
    label.set_wrap(true);

    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);

    dialog.content_area().append(&label);

    dialog.connect_response(|dialog, _| {
        dialog.close();
    });

    dialog.present();
}

fn copy_path(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir_all(source, destination)?;
    } else if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;

        #[cfg(unix)]
        symlink_unix(target, destination)?;
    } else {
        fs::copy(source, destination)?;
    }

    Ok(())
}

fn copy_dir_all(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())?;

            #[cfg(unix)]
            symlink_unix(link_target, target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}

fn move_path(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_path(source, destination)?;
                remove_all(source)?;
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn remove_all(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn unique_destination(destination: &Path) -> PathBuf {
    if !destination.exists() {
        return destination.to_path_buf();
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));

    let file_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let (stem, extension): (String, Option<String>) =
        if file_name.starts_with('.') && file_name.matches('.').count() == 1 {
            (file_name.clone(), None)
        } else {
            match file_name.rsplit_once('.') {
                Some((stem, extension)) => (stem.to_string(), Some(extension.to_string())),
                None => (file_name.clone(), None),
            }
        };

    let mut counter = 1;

    loop {
        let candidate = match &extension {
            Some(extension) => parent.join(format!("{stem} ({counter}).{extension}")),
            None => parent.join(format!("{stem} ({counter})")),
        };

        if !candidate.exists() {
            return candidate;
        }

        counter += 1;
    }
}
