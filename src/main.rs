#![allow(dead_code)]

mod app;
mod error;
mod filesystem;
mod navigation;
mod operations;
mod ui;

use gtk::prelude::*;
use gtk::glib;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox,
    Notebook, Orientation, ScrolledWindow, SearchEntry, SelectionMode,
};

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use app::context::AppContext;
use app::state::TabState;
use filesystem::directory;
use filesystem::metadata;
use navigation::bookmarks;
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

fn get_active_widgets(notebook: &Notebook) -> Option<(Rc<RefCell<TabState>>, ListBox)> {
    let page_num = notebook.current_page()?;
    let widget = notebook.nth_page(page_num)?;
    let state = widget.data::<Rc<RefCell<TabState>>>("tab-state")?.clone();
    let list = widget.data::<ListBox>("list-box")?.clone();
    Some((state, list))
}

fn normalize(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn navigate_to(tab_state: &Rc<RefCell<TabState>>, requested: PathBuf) {
    let path = normalize(requested);
    if !path.is_dir() {
        return;
    }
    let mut s = tab_state.borrow_mut();
    if s.current != path {
        s.history.push(s.current.clone());
        s.current = path;
    }
}

fn go_back(tab_state: &Rc<RefCell<TabState>>) {
    let previous = tab_state.borrow_mut().history.pop();
    if let Some(previous) = previous {
        tab_state.borrow_mut().current = previous;
    }
}

fn refresh_tab(
    tab_state: &Rc<RefCell<TabState>>,
    list: &ListBox,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
) {
    let mut s = tab_state.borrow_mut();
    let current = s.current.clone();

    // Sync toolbar without triggering infinite loops
    if location_entry.text().as_str() != current.display().to_string() {
        location_entry.set_text(&current.display().to_string());
    }
    if search_entry.text().as_str() != s.search_query {
        search_entry.set_text(&s.search_query);
    }
    if hidden_toggle.is_active() != s.show_hidden {
        hidden_toggle.set_active(s.show_hidden);
    }

    file_view::clear(list);

    let mut items = directory::read_items(&current, s.show_hidden);

    // Real-time Search Filter
    if !s.search_query.is_empty() {
        let q = s.search_query.to_lowercase();
        items.retain(|item| item.name.to_lowercase().contains(&q));
    }

    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    file_view::render(list, &items);
    s.items = items;

    sidebar::build(sidebar_list, &ctx.borrow().bookmarks);
}

fn add_tab(
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    path: PathBuf,
    window: &ApplicationWindow,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
) {
    let tab_state = Rc::new(RefCell::new(TabState::new(path.clone())));

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Multiple);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    let page_widget = GtkBox::new(Orientation::Vertical, 0);
    page_widget.append(&scrolled);

    page_widget.set_data("tab-state", tab_state.clone());
    page_widget.set_data("list-box", list.clone());

    // Tab Label with Close Button
    let tab_label = GtkBox::new(Orientation::Horizontal, 4);
    let label_text = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let label = Label::new(Some(&label_text));

    let close_btn = Button::new();
    close_btn.set_label("x");
    close_btn.set_has_frame(false);

    tab_label.append(&label);
    tab_label.append(&close_btn);

    let page_index = notebook.append_page(&page_widget, Some(&tab_label));
    notebook.set_tab_reorderable(&page_widget, true);

    {
        let notebook = notebook.clone();
        let page_widget = page_widget.clone();
        close_btn.connect_clicked(move |_| {
            if let Some(page_num) = notebook.page_num(&page_widget) {
                notebook.remove_page(page_num);
            }
        });
    }

    // --- Controllers ---
    
    // 1. Row Activated (Double Click / Enter)
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        list.connect_row_activated(move |_, row| {
            let index = row.index();
            if index < 0 {
                return;
            }

            if let Some((tab_state, _)) = get_active_widgets(&notebook) {
                let item = tab_state.borrow().items.get(index as usize).cloned();
                if let Some(item) = item {
                    if item.is_dir {
                        navigate_to(&tab_state, item.path);
                        refresh_tab(
                            &tab_state,
                            &list,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                    } else {
                        let _ = Command::new("xdg-open").arg(&item.path).spawn();
                    }
                }
            }
        });
    }

    // 2. Middle Click (Open in New Tab)
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        let middle_click = gtk::GestureClick::new();
        middle_click.set_button(2);
        middle_click.connect_pressed(move |_, _, _, y| {
            if let Some(row) = list.row_at_y(y as i32) {
                let index = row.index();
                if index >= 0 {
                    if let Some((tab_state, _)) = get_active_widgets(&notebook) {
                        let item = tab_state.borrow().items.get(index as usize).cloned();
                        if let Some(item) = item {
                            if item.is_dir {
                                add_tab(
                                    &notebook,
                                    &ctx,
                                    item.path,
                                    &window,
                                    &location_entry,
                                    &search_entry,
                                    &hidden_toggle,
                                    &sidebar_list,
                                );
                            }
                        }
                    }
                }
            }
        });
        list.add_controller(middle_click);
    }

    // 3. Drag Source
    {
        let state = tab_state.clone();
        let list = list.clone();

        let drag_source = gtk::DragSource::new();
        drag_source.connect_prepare(move |_source, _x, _y| {
            let selected = {
                let s = state.borrow();
                file_view::selected_items(&list, &s.items)
            };
            if selected.is_empty() {
                return None;
            }
            let payload = selected
                .iter()
                .map(|item| item.path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let provider = gtk::gdk::ContentProvider::new_value(&payload);
            Some((provider, gtk::gdk::DragAction::MOVE))
        });
        list.add_controller(drag_source);
    }

    // 4. Drop Target
    {
        let window_error = window.clone();
        let state = tab_state.clone();
        let list = list.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        let string_type = <String as glib::StaticType>::static_type();
        let drop_target = gtk::DropTarget::new(
            string_type,
            gtk::gdk::DragAction::MOVE | gtk::gdk::DragAction::COPY,
        );

        drop_target.connect_drop(move |_target, value, _x, y| {
            let Ok(payload) = value.get::<String>() else {
                return false;
            };
            let sources: Vec<PathBuf> = payload.lines().map(PathBuf::from).collect();
            if sources.is_empty() {
                return false;
            }

            if let Some(row) = list.row_at_y(y as i32) {
                let index = row.index();
                if index >= 0 {
                    let destination_item = {
                        let s = state.borrow();
                        s.items.get(index as usize).cloned()
                    };

                    if let Some(destination_item) = destination_item {
                        if destination_item.is_dir {
                            let mut moved = 0;
                            let mut skipped = 0;
                            let mut failed = 0;
                            let mut last_error = None;

                            for source in &sources {
                                if !source.exists()
                                    || source.as_path() == destination_item.path.as_path()
                                    || destination_item.path.starts_with(source)
                                    || source.parent() == Some(destination_item.path.as_path())
                                {
                                    skipped += 1;
                                    continue;
                                }

                                let file_name =
                                    source.file_name().unwrap_or_default().to_os_string();
                                let destination = operations::unique_destination(
                                    &destination_item.path.join(file_name),
                                );

                                match operations::move_op::move_path(source, &destination) {
                                    Ok(_) => moved += 1,
                                    Err(err) => {
                                        failed += 1;
                                        last_error = Some(err);
                                    }
                                }
                            }

                            if failed == 0 && moved > 0 {
                                // status update handled by refresh
                            } else if moved == 0 {
                                if let Some(err) = last_error {
                                    dialogs::show_error(&window_error, &format!("Move failed: {err}"));
                                }
                            }

                            refresh_tab(
                                &state,
                                &list,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                            );
                            return true;
                        }
                    }
                }
            }
            false
        });
        list.add_controller(drop_target);
    }

    // 5. Right Click Context Menu
    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

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
                        let s = tab_state.borrow();
                        file_view::selected_items(&list, &s.items)
                    };

                    if !items.is_empty() {
                        show_context_menu(
                            &window,
                            &notebook,
                            &ctx,
                            &list,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                            items,
                            x,
                            y,
                        );
                    }
                }
            }
        });
        list.add_controller(right_click);
    }

    // Initial Refresh
    refresh_tab(
        &tab_state,
        &list,
        ctx,
        location_entry,
        search_entry,
        hidden_toggle,
        sidebar_list,
    );

    notebook.set_current_page(Some(page_index));
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

    let ctx = Rc::new(RefCell::new(AppContext::new()));

    // Toolbar Row 1
    let toolbar1 = GtkBox::new(Orientation::Horizontal, 6);
    let back_btn = Button::with_label("Back");
    let up_btn = Button::with_label("Up");
    let home_btn = Button::with_label("Home");
    let bookmark_btn = Button::with_label("Bookmark");
    
    let location_entry = Entry::new();
    location_entry.set_placeholder_text(Some("/path/to/directory"));
    location_entry.set_hexpand(true);
    
    let search_entry = SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search..."));
    search_entry.set_width_request(200);

    toolbar1.append(&back_btn);
    toolbar1.append(&up_btn);
    toolbar1.append(&home_btn);
    toolbar1.append(&bookmark_btn);
    toolbar1.append(&location_entry);
    toolbar1.append(&search_entry);

    // Toolbar Row 2
    let toolbar2 = GtkBox::new(Orientation::Horizontal, 6);
    let new_folder_btn = Button::with_label("New Folder");
    let new_file_btn = Button::with_label("New File");
    let rename_btn = Button::with_label("Rename");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let paste_btn = Button::with_label("Paste");
    let trash_btn = Button::with_label("Trash");
    let hidden_toggle = CheckButton::with_label("Hidden");

    toolbar2.append(&new_folder_btn);
    toolbar2.append(&new_file_btn);
    toolbar2.append(&rename_btn);
    toolbar2.append(&copy_btn);
    toolbar2.append(&move_btn);
    toolbar2.append(&paste_btn);
    toolbar2.append(&trash_btn);
    toolbar2.append(&hidden_toggle);

    // Sidebar
    let sidebar_list = ListBox::new();
    sidebar_list.set_selection_mode(SelectionMode::Single);
    let sidebar_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    sidebar_scrolled.set_child(Some(&sidebar_list));
    sidebar_scrolled.set_width_request(190);
    sidebar_scrolled.set_vexpand(true);

    // Notebook (Tabs)
    let notebook = Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_show_border(false);
    notebook.set_vexpand(true);
    notebook.set_hexpand(true);

    let content = GtkBox::new(Orientation::Horizontal, 6);
    content.append(&sidebar_scrolled);
    content.append(&notebook);
    content.set_vexpand(true);

    root.append(&toolbar1);
    root.append(&toolbar2);
    root.append(&content);

    window.set_child(Some(&root));

    // Create initial tab
    add_tab(
        &notebook,
        &ctx,
        locations::home_dir(),
        &window,
        &location_entry,
        &search_entry,
        &hidden_toggle,
        &sidebar_list,
    );

    // --- Global Signals ---

    // Tab Switch
    {
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        notebook.connect_switch_page(move |nb, _, _| {
            if let Some((tab_state, list)) = get_active_widgets(nb) {
                refresh_tab(
                    &tab_state,
                    &list,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                );
            }
        });
    }

    // Search Entry
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry_clone = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        search_entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                if tab_state.borrow().search_query != query {
                    tab_state.borrow_mut().search_query = query;
                    refresh_tab(
                        &tab_state,
                        &list,
                        &ctx,
                        &location_entry,
                        &search_entry_clone,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                }
            }
        });
    }

    // Location Entry
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry_clone = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        location_entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            let path = PathBuf::from(text);
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                if path.is_dir() {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &list,
                        &ctx,
                        &location_entry_clone,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                }
            }
        });
    }

    // Sidebar Click
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        sidebar_list.connect_row_activated(move |_, row| {
            if let Some(path) = sidebar::resolve_click(row) {
                if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &list,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                }
            }
        });
    }

    // Sidebar Right Click
    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            if let Some(row) = sidebar_list.row_at_y(y as i32) {
                if let Some(name) = row.name() {
                    if let Some(path_str) = name.strip_prefix("bm:") {
                        let path = PathBuf::from(path_str);
                        show_sidebar_context_menu(
                            &window,
                            &notebook,
                            &ctx,
                            &sidebar_list,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            path,
                            x,
                            y,
                        );
                    }
                }
            }
        });
        sidebar_list.add_controller(right_click);
    }

    // Toolbar Buttons
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        back_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                go_back(&tab_state);
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        up_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let current = tab_state.borrow().current.clone();
                if let Some(parent) = current.parent() {
                    navigate_to(&tab_state, parent.to_path_buf());
                    refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                }
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        home_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                navigate_to(&tab_state, locations::home_dir());
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        hidden_toggle.connect_toggled(move |toggle| {
            let is_active = toggle.is_active();
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                if tab_state.borrow().show_hidden != is_active {
                    tab_state.borrow_mut().show_hidden = is_active;
                    refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                }
            }
        });
    }

    {
        let window_parent = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        new_folder_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                dialogs::show_text_dialog(&window_parent, "New Folder", "New Folder", "Create", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let list = list.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();

                    move |name| {
                        if name.is_empty() { return; }
                        let parent = tab_state.borrow().current.clone();
                        if let Err(err) = operations::create::create_folder(&parent, &name) {
                            dialogs::show_error(&window_error, &format!("Could not create folder: {err}"));
                        }
                        refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                    }
                });
            }
        });
    }

    {
        let window_parent = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        new_file_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                dialogs::show_text_dialog(&window_parent, "New File", "new-file.txt", "Create", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let list = list.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();

                    move |name| {
                        if name.is_empty() { return; }
                        let parent = tab_state.borrow().current.clone();
                        if let Err(err) = operations::create::create_file(&parent, &name) {
                            dialogs::show_error(&window_error, &format!("Could not create file: {err}"));
                        }
                        refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                    }
                });
            }
        });
    }

    {
        let window_parent = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        rename_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let selected = {
                    let s = tab_state.borrow();
                    file_view::selected_items(&list, &s.items)
                };
                if selected.len() != 1 { return; }

                let item = selected[0].clone();
                let source = item.path.clone();
                let initial_name = item.name.clone();

                dialogs::show_text_dialog(&window_parent, "Rename", &initial_name, "Rename", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let list = list.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();

                    move |name| {
                        if name.is_empty() { return; }
                        if let Err(err) = operations::rename::rename_path(&source, &name) {
                            dialogs::show_error(&window_error, &format!("Could not rename: {err}"));
                        }
                        refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                    }
                });
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();

        copy_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let selected = {
                    let s = tab_state.borrow();
                    file_view::selected_items(&list, &s.items)
                };
                if selected.is_empty() { return; }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.path.clone()).collect();
                ctx.borrow_mut().pending = Some((PendingOp::Copy, paths));
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();

        move_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let selected = {
                    let s = tab_state.borrow();
                    file_view::selected_items(&list, &s.items)
                };
                if selected.is_empty() { return; }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.path.clone()).collect();
                ctx.borrow_mut().pending = Some((PendingOp::Move, paths));
            }
        });
    }

    {
        let window_error = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        paste_btn.connect_clicked(move |_| {
            let pending = ctx.borrow_mut().pending.take();
            let Some((operation, sources)) = pending else { return; };

            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let destination_dir = tab_state.borrow().current.clone();
                match operations::paste_pending(&destination_dir, operation, &sources) {
                    Ok(_) => {}
                    Err(err) => {
                        dialogs::show_error(&window_error, &format!("Paste failed: {err}"));
                    }
                }
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let window_error = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        trash_btn.connect_clicked(move |_| {
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                let selected = {
                    let s = tab_state.borrow();
                    file_view::selected_items(&list, &s.items)
                };
                if selected.is_empty() { return; }

                let mut last_error = None;
                for item in &selected {
                    if let Err(err) = operations::trash::delete(&item.path) {
                        last_error = Some(err);
                    }
                }
                if let Some(err) = last_error {
                    dialogs::show_error(&window_error, &format!("Could not move to trash: {err}"));
                }
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let sidebar_list = sidebar_list.clone();

        bookmark_btn.connect_clicked(move |_| {
            if let Some((tab_state, _)) = get_active_widgets(&notebook) {
                let mut c = ctx.borrow_mut();
                let current = tab_state.borrow().current.clone();
                let name = current.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| current.to_string_lossy().to_string());

                if c.bookmarks.iter().any(|b| b.path == current) { return; }

                bookmarks::add(&mut c.bookmarks, name, current);
                drop(c);
                sidebar::build(&sidebar_list, &ctx.borrow().bookmarks);
            }
        });
    }

    window.present();
}

fn show_context_menu(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    list: &ListBox,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    items: Vec<directory::Item>,
    x: f64,
    y: f64,
) {
    if items.is_empty() { return; }

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
    let open_tab_btn = Button::with_label("Open in New Tab");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let rename_btn = Button::with_label("Rename");
    let trash_btn = Button::with_label("Trash");
    let properties_btn = Button::with_label("Properties");

    open_btn.set_sensitive(count == 1);
    open_tab_btn.set_sensitive(count == 1 && single_item.as_ref().map_or(false, |i| i.is_dir));
    rename_btn.set_sensitive(count == 1);
    properties_btn.set_sensitive(count == 1);

    menu_box.append(&open_btn);
    menu_box.append(&open_tab_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&move_btn);
    menu_box.append(&rename_btn);
    menu_box.append(&trash_btn);
    menu_box.append(&properties_btn);

    popover.set_child(Some(&menu_box));

    {
        let popover = popover.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let item = single_item.clone();

        open_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = item.clone() else { return; };
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                if item.is_dir {
                    navigate_to(&tab_state, item.path.clone());
                    refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                } else {
                    let _ = Command::new("xdg-open").arg(&item.path).spawn();
                }
            }
        });
    }

    {
        let popover = popover.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        open_tab_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = single_item.clone() else { return; };
            if item.is_dir {
                add_tab(&notebook, &ctx, item.path, &window, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let popover = popover.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();

        copy_btn.connect_clicked(move |_| {
            popover.popdown();
            ctx.borrow_mut().pending = Some((PendingOp::Copy, paths.clone()));
        });
    }

    {
        let popover = popover.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();

        move_btn.connect_clicked(move |_| {
            popover.popdown();
            ctx.borrow_mut().pending = Some((PendingOp::Move, paths.clone()));
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let single_item = single_item.clone();

        rename_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = single_item.clone() else { return; };
            let source = item.path.clone();
            let initial_name = item.name.clone();

            dialogs::show_text_dialog(&window, "Rename", &initial_name, "Rename", {
                let window = window.clone();
                let notebook = notebook.clone();
                let ctx = ctx.clone();
                let location_entry = location_entry.clone();
                let search_entry = search_entry.clone();
                let hidden_toggle = hidden_toggle.clone();
                let sidebar_list = sidebar_list.clone();

                move |name| {
                    if name.is_empty() { return; }
                    if let Err(err) = operations::rename::rename_path(&source, &name) {
                        dialogs::show_error(&window, &format!("Could not rename: {err}"));
                    }
                    if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                        refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
                    }
                }
            });
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();

        trash_btn.connect_clicked(move |_| {
            popover.popdown();
            let mut last_error = None;
            for path in &paths {
                if let Err(err) = operations::trash::delete(path) {
                    last_error = Some(err);
                }
            }
            if let Some(err) = last_error {
                dialogs::show_error(&window, &format!("Could not move to trash: {err}"));
            }
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let item = single_item.clone();

        properties_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = item.clone() else { return; };

            let size = if item.is_dir { "-".to_string() } else { metadata::format_size(item.metadata.size) };
            let modified = metadata::format_modified(item.metadata.modified);
            let permissions = item.metadata.permissions.clone();
            let is_symlink = item.metadata.is_symlink;
            let kind = if item.is_dir { "Folder" } else { "File" };

            let message = format!(
                "Name: {}\nPath: {}\nType: {}\nSize: {}\nModified: {}\nPermissions: {}\nSymlink: {}",
                item.name, item.path.display(), kind, size, modified, permissions, is_symlink
            );

            dialogs::show_info(&window, "Properties", &message);
        });
    }

    popover.popup();
}

fn show_sidebar_context_menu(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    sidebar_list: &ListBox,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    path: PathBuf,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(sidebar_list);
    popover.set_pointing_to(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let remove_btn = Button::with_label("Remove Bookmark");
    menu_box.append(&remove_btn);
    popover.set_child(Some(&menu_box));

    {
        let popover = popover.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let sidebar_list = sidebar_list.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();

        remove_btn.connect_clicked(move |_| {
            popover.popdown();
            bookmarks::remove(&mut ctx.borrow_mut().bookmarks, &path);
            sidebar::build(&sidebar_list, &ctx.borrow().bookmarks);
            
            if let Some((tab_state, list)) = get_active_widgets(&notebook) {
                refresh_tab(&tab_state, &list, &ctx, &location_entry, &search_entry, &hidden_toggle, &sidebar_list);
            }
        });
    }

    popover.popup();
}
