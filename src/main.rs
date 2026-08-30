#![allow(dead_code)]

mod app;
mod config;
mod error;
mod filesystem;
mod mime;
mod navigation;
mod operations;
mod plugins;
mod portal;
mod search;
mod ui;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox,
    Notebook, Orientation, ScrolledWindow, SearchEntry, SelectionMode,
};

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use app::context::AppContext;
use app::state::TabState;
use filesystem::directory;
use filesystem::metadata;
use navigation::bookmarks;
use navigation::locations;
use operations::PendingOp;
use ui::dialogs;
use ui::grid_view;
use ui::item_object::ItemObject;
use ui::sidebar;

static VIEW_MODE_LIST: AtomicBool = AtomicBool::new(false);

thread_local! {
    static TYPEAHEAD_BUFFER: std::cell::RefCell<(String, std::time::Instant)> =
        std::cell::RefCell::new((String::new(), std::time::Instant::now()));
}

enum JobRequest {
    Paste {
        operation: PendingOp,
        tasks: Vec<operations::jobs::PasteTask>,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    CompressZip {
        sources: Vec<PathBuf>,
        archive_path: PathBuf,
    },
    ExtractArchive {
        archive_path: PathBuf,
        destination_dir: PathBuf,
    },
    BatchRename {
        renames: Vec<(PathBuf, PathBuf)>,
    },
}

struct JobQueueState {
    pending: VecDeque<JobRequest>,
    running: bool,
}

type JobQueue = Rc<RefCell<JobQueueState>>;

#[derive(Clone)]
struct JobUi {
    window: ApplicationWindow,
    notebook: Notebook,
    ctx: Rc<RefCell<AppContext>>,
    location_entry: Entry,
    search_entry: SearchEntry,
    hidden_toggle: CheckButton,
    sidebar_list: ListBox,
    watcher_manager: Rc<RefCell<filesystem::watcher::WatcherManager>>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let app = Application::builder()
        .application_id("org.mitos.file-manager")
        .build();

    app.connect_activate(move |app| {
        build_ui(app, &args);
    });

    let _ = app.run();
}

fn get_active_widgets(
    notebook: &Notebook,
) -> Option<(
    Rc<RefCell<TabState>>,
    gtk::GridView,
    gio::ListStore,
    gtk::MultiSelection,
)> {
    let page_num = notebook.current_page()?;
    let widget = notebook.nth_page(page_num)?;
    let state = widget.data::<Rc<RefCell<TabState>>>("tab-state")?.clone();
    let grid = widget.data::<gtk::GridView>("grid-view")?.clone();
    let store = widget.data::<gio::ListStore>("list-store")?.clone();
    let selection = widget
        .data::<gtk::MultiSelection>("selection-model")?
        .clone();
    Some((state, grid, store, selection))
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

fn update_watcher(
    notebook: &Notebook,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
) {
    if let Some((tab_state, _, _, _)) = get_active_widgets(notebook) {
        let current = tab_state.borrow().current.clone();
        watcher_manager.borrow_mut().watch(&current);
    }
}

fn open_file_default(path: &PathBuf) {
    let uri = format!("file://{}", path.display());

    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

fn start_paste_job_ui(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    operation: PendingOp,
    sources: Vec<PathBuf>,
    destination: PathBuf,
) {
    if sources.is_empty() {
        return;
    }

    let tasks = prepare_paste_tasks(window, sources, destination);
    if tasks.is_empty() {
        return;
    }

    let Some(queue) = location_entry
        .data::<JobQueue>("job-queue")
        .map(|q| q.clone())
    else {
        return;
    };

    let ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    enqueue_job(&queue, JobRequest::Paste { operation, tasks }, ui);
}

fn start_trash_job_ui(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    paths: Vec<PathBuf>,
) {
    if paths.is_empty() {
        return;
    }

    let Some(queue) = location_entry
        .data::<JobQueue>("job-queue")
        .map(|q| q.clone())
    else {
        return;
    };

    let ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    enqueue_job(&queue, JobRequest::Trash { paths }, ui);
}

fn start_compress_zip_job_ui(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    sources: Vec<PathBuf>,
    destination_dir: PathBuf,
) {
    if sources.is_empty() {
        return;
    }

    let archive_path = operations::archive::default_archive_path(&destination_dir);

    let Some(queue) = location_entry
        .data::<JobQueue>("job-queue")
        .map(|q| q.clone())
    else {
        return;
    };

    let ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    enqueue_job(
        &queue,
        JobRequest::CompressZip {
            sources,
            archive_path,
        },
        ui,
    );
}

fn start_extract_archive_job_ui(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    archive_path: PathBuf,
    destination_dir: PathBuf,
) {
    if !archive_path.exists() {
        return;
    }

    let extract_dir = operations::archive::default_extract_dir(&destination_dir, &archive_path);

    let Some(queue) = location_entry
        .data::<JobQueue>("job-queue")
        .map(|q| q.clone())
    else {
        return;
    };

    let ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    enqueue_job(
        &queue,
        JobRequest::ExtractArchive {
            archive_path,
            destination_dir: extract_dir,
        },
        ui,
    );
}

fn start_batch_rename_job_ui(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    renames: Vec<(PathBuf, PathBuf)>,
) {
    if renames.is_empty() {
        return;
    }

    let Some(queue) = location_entry
        .data::<JobQueue>("job-queue")
        .map(|q| q.clone())
    else {
        return;
    };

    let ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    enqueue_job(&queue, JobRequest::BatchRename { renames }, ui);
}

fn prepare_paste_tasks(
    window: &ApplicationWindow,
    sources: Vec<PathBuf>,
    destination: PathBuf,
) -> Vec<operations::jobs::PasteTask> {
    use operations::jobs::{ConflictAction, ConflictPolicy, PasteTask};

    let mut has_conflict = false;

    for source in &sources {
        let Some(file_name) = source.file_name() else {
            continue;
        };

        if destination.join(file_name).exists() {
            has_conflict = true;
            break;
        }
    }

    let policy = if has_conflict {
        dialogs::choose_conflict_policy(window)
    } else {
        Some(ConflictPolicy::KeepBoth)
    };

    let Some(policy) = policy else {
        return Vec::new();
    };

    let mut tasks = Vec::new();

    for source in sources {
        let Some(file_name) = source.file_name() else {
            continue;
        };

        let base = destination.join(file_name);
        let exists = base.exists();

        let action = match policy {
            ConflictPolicy::KeepBoth => ConflictAction::KeepBoth,
            ConflictPolicy::Replace => ConflictAction::Replace,
            ConflictPolicy::SkipExisting => {
                if exists {
                    ConflictAction::Skip
                } else {
                    ConflictAction::KeepBoth
                }
            }
        };

        if action != ConflictAction::Skip {
            tasks.push(PasteTask {
                source,
                destination: base,
                action,
            });
        }
    }

    tasks
}

fn enqueue_job(queue: &JobQueue, request: JobRequest, ui: JobUi) {
    let (start_now, pending_count) = {
        let mut q = queue.borrow_mut();
        q.pending.push_back(request);

        if !q.running {
            q.running = true;
            (true, q.pending.len())
        } else {
            (false, q.pending.len())
        }
    };

    if start_now {
        start_next_job(queue, ui);
    } else if let Some(status_label) = ui.location_entry.data::<Label>("status-label") {
        status_label.set_label(&format!("{pending_count} job(s) queued"));
    }
}

fn start_next_job(queue: &JobQueue, ui: JobUi) {
    let request = { queue.borrow_mut().pending.pop_front() };

    let Some(request) = request else {
        queue.borrow_mut().running = false;
        return;
    };

    let (sender, receiver) = async_channel::unbounded();

    let (handle, title) = match request {
        JobRequest::Paste { operation, tasks } => {
            let handle = operations::jobs::start_paste_job(operation, tasks, sender);
            (handle, "File Operation")
        }
        JobRequest::Trash { paths } => {
            let handle = operations::jobs::start_trash_job(paths, sender);
            (handle, "Trash")
        }
        JobRequest::CompressZip {
            sources,
            archive_path,
        } => {
            let handle = operations::archive::start_compress_zip_job(sources, archive_path, sender);
            (handle, "Compress")
        }
        JobRequest::ExtractArchive {
            archive_path,
            destination_dir,
        } => {
            let handle =
                operations::archive::start_extract_job(archive_path, destination_dir, sender);
            (handle, "Extract")
        }
        JobRequest::BatchRename { renames } => {
            let handle = operations::batch_rename::start_batch_rename_job(renames, sender);
            (handle, "Batch Rename")
        }
    };

    let queue_for_done = queue.clone();
    let ui_for_done = ui.clone();

    let window_error = ui.window.clone();
    let notebook = ui.notebook.clone();
    let ctx = ui.ctx.clone();
    let location_entry = ui.location_entry.clone();
    let search_entry = ui.search_entry.clone();
    let hidden_toggle = ui.hidden_toggle.clone();
    let sidebar_list = ui.sidebar_list.clone();
    let watcher_manager = ui.watcher_manager.clone();

    ui::progress::show_progress_dialog(&ui.window, title, handle, receiver, move |result| {
        // Native desktop notification via the session notification daemon
        match &result {
            Ok(count) => send_job_notification(
                &window_error,
                "Operation complete",
                &format!("{title}: {count} item(s) processed"),
            ),
            Err(err) if err.contains("Cancelled") => {
                send_job_notification(&window_error, "Operation cancelled", title);
            }
            Err(err) => {
                send_job_notification(
                    &window_error,
                    "Operation failed",
                    &format!("{title}: {err}"),
                );
            }
        }

        if let Err(err) = result {
            if !err.contains("Cancelled") {
                dialogs::show_error(&window_error, &format!("Job failed: {err}"));
            }
        }

        if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
            refresh_tab(
                &tab_state,
                &store,
                &ctx,
                &location_entry,
                &search_entry,
                &hidden_toggle,
                &sidebar_list,
            );

            update_watcher(&notebook, &watcher_manager);
        }

        start_next_job(&queue_for_done, ui_for_done);
    });
}

fn refresh_tab(
    tab_state: &Rc<RefCell<TabState>>,
    store: &gio::ListStore,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
) {
    let mut s = tab_state.borrow_mut();
    let current = s.current.clone();

    if let Some(crumbs) = location_entry.data::<GtkBox>("path-crumbs") {
        ui::path_bar::update(&crumbs, location_entry, &current);
    } else if location_entry.text().as_str() != current.display().to_string() {
        location_entry.set_text(&current.display().to_string());
    }

    if search_entry.text().as_str() != s.search_query {
        search_entry.set_text(&s.search_query);
    }

    if hidden_toggle.is_active() != s.show_hidden {
        hidden_toggle.set_active(s.show_hidden);
    }

    let mut items = directory::read_items(&current, s.show_hidden);

    if !s.search_query.is_empty() {
        let q = s.search_query.to_lowercase();
        items.retain(|item| item.name.to_lowercase().contains(&q));
    }

    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    let item_count = items.len();

    grid_view::render(store, &items);
    s.items = items;

    if let Some(stack) = store.data::<gtk::Stack>("view-stack") {
        if let Some(empty_widget) = stack.child_by_name("empty") {
            if let Some(empty_lbl) = empty_widget.downcast_ref::<Label>() {
                empty_lbl.set_label(if s.search_query.is_empty() {
                    "This folder is empty"
                } else {
                    "No results found"
                });
            }
        }

        let mode = if VIEW_MODE_LIST.load(Ordering::Relaxed) {
            "list"
        } else {
            "files"
        };

        stack.set_visible_child_name(if item_count == 0 { "empty" } else { mode });
    }

    if let Some(win) = location_entry.data::<gtk::ApplicationWindow>("main-window") {
        sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
    }

    if let Some(status_label) = location_entry.data::<Label>("status-label") {
        let free = filesystem_free_string(&current);

        status_label.set_label(&format!(
            "{} · {} items · {} free",
            current.display(),
            item_count,
            free
        ));
    }
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
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
) {
    let tab_state = Rc::new(RefCell::new(TabState::new(path.clone())));
    let (store, selection) = grid_view::create_model();
    let grid = grid_view::create_grid_view(&selection);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scrolled.set_child(Some(&grid));
    scrolled.set_vexpand(true);

    let empty_label = Label::new(Some("This folder is empty"));
    empty_label.set_vexpand(true);
    empty_label.set_valign(gtk::Align::Center);
    empty_label.add_css_class("dim-label");

    let view_stack = gtk::Stack::new();
    view_stack.set_vexpand(true);
    view_stack.add_named(&scrolled, Some("files"));
    view_stack.add_named(&empty_label, Some("empty"));

    let list_view = ui::list_view::create_list_view(&selection);
    view_stack.add_named(&list_view, Some("list"));

    let page_widget = GtkBox::new(Orientation::Vertical, 0);
    page_widget.append(&view_stack);

    store.set_data("view-stack", view_stack.clone());

    page_widget.set_data("tab-state", tab_state.clone());
    page_widget.set_data("grid-view", grid.clone());
    page_widget.set_data("list-store", store.clone());
    page_widget.set_data("selection-model", selection.clone());

    // Update selection status
    {
        let selection_label = location_entry
            .data::<Label>("selection-label")
            .map(|l| l.clone());

        let preview_box = location_entry
            .data::<GtkBox>("preview-box")
            .map(|p| p.clone());

        let store_for_selection = store.clone();

        selection.connect_selection_changed(move |selection| {
            let selected = grid_view::selected_items(selection, &store_for_selection);

            if let Some(selection_label) = selection_label.clone() {
                if selected.is_empty() {
                    selection_label.set_label("");
                } else {
                    let total: u64 = selected.iter().map(|item| item.size()).sum();

                    selection_label.set_label(&format!(
                        "{} selected · {}",
                        selected.len(),
                        metadata::format_size(total)
                    ));
                }
            }

            if let Some(preview_box) = preview_box.clone() {
                ui::preview::update(&preview_box, selected.first());
            }
        });
    }

    let tab_label = GtkBox::new(Orientation::Horizontal, 4);
    let label_text = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let label = Label::new(Some(&label_text));
    let close_btn = Button::with_label("x");
    close_btn.set_has_frame(false);
    tab_label.append(&label);
    tab_label.append(&close_btn);

    let page_index = notebook.append_page(&page_widget, Some(&tab_label));
    notebook.set_tab_reorderable(&page_widget, true);

    // Tab context menu (right-click on tab)
    {
        let notebook = notebook.clone();
        let page_widget = page_widget.clone();

        let tab_gesture = gtk::GestureClick::new();
        tab_gesture.set_button(3);

        tab_gesture.connect_pressed(move |_gesture, _n_press, x, y| {
            ui::tab_menu::show_tab_context_menu(&notebook, &page_widget, x, y);
        });

        tab_label.add_controller(tab_gesture);
    }

    {
        let notebook = notebook.clone();
        let page_widget = page_widget.clone();
        close_btn.connect_clicked(move |_| {
            if let Some(page_num) = notebook.page_num(&page_widget) {
                notebook.remove_page(page_num);
            }
        });
    }

    // 1. Row Activated
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let window = window.clone();

        grid.connect_activate(move |_, pos| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                if let Some(obj) = store.item(pos) {
                    if let Some(item_obj) = obj.downcast_ref::<ItemObject>() {
                        if item_obj.is_dir() {
                            navigate_to(&tab_state, item_obj.get_path());
                            refresh_tab(
                                &tab_state,
                                &store,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                            );
                            update_watcher(&notebook, &watcher_manager);
                        } else {
                            let path = item_obj.get_path();
                            let mime = item_obj.mime_type();

                            if let Some(app) =
                                crate::mime::applications::default_app_for_mime(&mime)
                            {
                                if let Err(err) =
                                    crate::mime::applications::launch_app_with_file(&app, &path)
                                {
                                    crate::ui::dialogs::show_error(
                                        &window,
                                        &format!("Failed to open: {}", err),
                                    );
                                }
                            } else {
                                let path = item_obj.get_path();
                                open_file_default(&path);
                            }
                        }
                    }
                }
            }
        });
    }

    // 2. Drag Source
    {
        let selection = selection.clone();
        let store = store.clone();

        let drag_source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
            .build();

        drag_source.connect_prepare(move |_source, _x, _y| {
            let selected = grid_view::selected_items(&selection, &store);
            if selected.is_empty() {
                return None;
            }

            let files: Vec<gtk::gio::File> = selected
                .iter()
                .map(|item| gtk::gio::File::for_path(item.get_path()))
                .collect();

            let file_list = FileList::from_array(&files);
            let provider = gtk::gdk::ContentProvider::for_value(&file_list.to_value());
            Some(provider)
        });
        grid.add_controller(drag_source);
    }

    // 3. Drop Target
    {
        let window_error = window.clone();
        let tab_state = tab_state.clone();
        let store = store.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let notebook = notebook.clone();

        let drop_target = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY)

        drop_target.connect_drop(move |target, value, _x, _y| {
            let Ok(file_list) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };

            let action = target.current_action();
            let is_copy = action == gtk::gdk::DragAction::COPY;

            let files = file_list.files();
            if files.is_empty() {
                return false;
            }

            let destination_dir = tab_state.borrow().current.clone();

            let sources: Vec<PathBuf> = files
                .iter()
                .filter_map(|file| file.path().map(PathBuf::from))
                .collect();

            if sources.is_empty() {
                return false;
            }

            let operation = if is_copy {
                PendingOp::Copy
            } else {
                PendingOp::Move
            };

            start_paste_job_ui(
                &window_error,
                &notebook,
                &ctx,
                &location_entry,
                &search_entry,
                &hidden_toggle,
                &sidebar_list,
                &watcher_manager,
                operation,
                sources,
                destination_dir,
            );

            true
        });

        grid.add_controller(drop_target);
    }

    // 4. Right Click Context Menu
    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let store = store.clone();
        let selection = selection.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let items = grid_view::selected_items(&selection, &store);
            if !items.is_empty() {
                show_context_menu(
                    &window,
                    &notebook,
                    &ctx,
                    &grid,
                    &store,
                    &selection,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    items,
                    x,
                    y,
                );
            }
        });
        grid.add_controller(right_click);
    }

    // ---- List view: activate (double click / Enter) ----
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        list_view.connect_activate(move |view, pos| {
            let obj = view
                .model()
                .and_then(|m| m.item(pos))
                .and_then(|o| o.downcast::<ItemObject>().ok());

            if let Some(item_obj) = obj {
                if item_obj.is_dir() {
                    if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                        navigate_to(&tab_state, item_obj.get_path());
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                } else {
                    let path = item_obj.get_path();
                    open_file_default(&path);
                }
            }
        });
    }

    // ---- List view: right-click context menu ----
    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let store = store.clone();
        let selection = selection.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let list_view = list_view.clone();

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let items = grid_view::selected_items(&selection, &store);
            if !items.is_empty() {
                show_context_menu(
                    &window,
                    &notebook,
                    &ctx,
                    &list_view,
                    &store,
                    &selection,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    items,
                    x,
                    y,
                );
            }
        });

        list_view.add_controller(right_click);
    }

    // ---- List view: drag source ----
    {
        let selection = selection.clone();
        let store = store.clone();

        let drag_source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
            .build();

        drag_source.connect_prepare(move |_source, _x, _y| {
            let selected = grid_view::selected_items(&selection, &store);
            if selected.is_empty() {
                return None;
            }

            let files: Vec<gtk::gio::File> = selected
                .iter()
                .map(|i| gtk::gio::File::for_path(i.get_path()))
                .collect();

            let file_list = FileList::from_array(&files);
            Some(gtk::gdk::ContentProvider::for_value(&file_list.to_value()))
        });

        list_view.add_controller(drag_source);
    }

    // ---- List view: drop target (into current directory) ----
    {
        let window_error = window.clone();
        let tab_state = tab_state.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let notebook = notebook.clone();

        let drop_target = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY)

        drop_target.connect_drop(move |target, value, _x, _y| {
            let Ok(file_list) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };

            let is_copy = target.current_action() == gtk::gdk::DragAction::COPY;
            let files = file_list.files();
            if files.is_empty() {
                return false;
            }

            let destination_dir = tab_state.borrow().current.clone();
            let sources: Vec<PathBuf> = files
                .iter()
                .filter_map(|f| f.path().map(PathBuf::from))
                .collect();
            if sources.is_empty() {
                return false;
            }

            let operation = if is_copy {
                PendingOp::Copy
            } else {
                PendingOp::Move
            };

            start_paste_job_ui(
                &window_error,
                &notebook,
                &ctx,
                &location_entry,
                &search_entry,
                &hidden_toggle,
                &sidebar_list,
                &watcher_manager,
                operation,
                sources,
                destination_dir,
            );

            true
        });

        list_view.add_controller(drop_target);
    }

    refresh_tab(
        &tab_state,
        &store,
        ctx,
        location_entry,
        search_entry,
        hidden_toggle,
        sidebar_list,
    );
    notebook.set_current_page(Some(page_index));
    update_watcher(notebook, watcher_manager);
}

fn build_ui(app: &Application, initial_args: &[String]) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("MITOS Files")
        .default_width(1100)
        .default_height(720)
        .build();

    ui::accessibility::setup_widget_accessibility(&window);

    let root = GtkBox::new(Orientation::Vertical, 6);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let ctx = Rc::new(RefCell::new(AppContext::new()));

    // After loading settings
    config::settings::load();

    // Start config watcher
    let (config_tx, config_rx) = async_channel::unbounded();
    let _config_watcher = config::watcher::ConfigWatcher::start(config_tx);

    let theme_mode = config::settings::theme_mode();
    ui::theme::apply_theme(&window.display(), theme_mode);

    let portal_rx = portal::service::start();

    // Setup Inotify Channel
    let (sender, receiver) = async_channel::unbounded();
    let watcher_manager = Rc::new(RefCell::new(filesystem::watcher::WatcherManager::new(
        sender,
    )));

    let toolbar1 = GtkBox::new(Orientation::Horizontal, 6);
    let back_btn = Button::with_label("Back");
    let forward_btn = Button::with_label("Forward");
    let up_btn = Button::with_label("Up");
    let home_btn = Button::with_label("Home");
    let new_window_btn = Button::with_label("New Window");
    let bookmark_btn = Button::with_label("Bookmark");
    let help_btn = Button::with_label("?");

    let (location_bar, location_stack, path_crumbs, location_entry) = ui::path_bar::build();
    location_entry.set_placeholder_text(Some("/path/to/directory"));
    location_entry.set_hexpand(true);

    let search_entry = SearchEntry::new();
    search_entry.set_placeholder_text(Some("Search..."));
    search_entry.set_width_request(200);

    let search_recursive_toggle = CheckButton::with_label("Recursive");
    search_recursive_toggle.set_active(true);

    toolbar1.append(&back_btn);
    toolbar1.append(&forward_btn);
    toolbar1.append(&up_btn);
    toolbar1.append(&home_btn);
    toolbar1.append(&new_window_btn);
    toolbar1.append(&bookmark_btn);
    toolbar1.append(&location_bar);
    toolbar1.append(&search_entry);
    toolbar1.append(&search_recursive_toggle);
    toolbar1.append(&help_btn);

    let toolbar2 = GtkBox::new(Orientation::Horizontal, 6);
    let new_folder_btn = Button::with_label("New Folder");
    let new_file_btn = Button::with_label("New File");
    let rename_btn = Button::with_label("Rename");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let list_toggle = CheckButton::with_label("List");
    let paste_btn = Button::with_label("Paste");
    let trash_btn = Button::with_label("Trash");
    let open_trash_btn = Button::with_label("Open Trash");
    let settings_btn = Button::with_label("Settings");
    let preview_toggle = CheckButton::with_label("Preview");
    let split_toggle = CheckButton::with_label("Split View");
    let tree_toggle = CheckButton::with_label("Tree");
    let hidden_toggle = CheckButton::with_label("Hidden");

    toolbar2.append(&new_folder_btn);
    toolbar2.append(&new_file_btn);
    toolbar2.append(&rename_btn);
    toolbar2.append(&copy_btn);
    toolbar2.append(&move_btn);
    toolbar2.append(&paste_btn);
    toolbar2.append(&trash_btn);
    toolbar2.append(&open_trash_btn);
    toolbar2.append(&settings_btn);
    toolbar2.append(&preview_toggle);
    toolbar2.append(&list_toggle);
    toolbar2.append(&split_toggle);
    toolbar2.append(&tree_toggle);
    toolbar2.append(&hidden_toggle);

    // Accessibility Tooltips
    back_btn.set_tooltip_text(Some("Go back (Alt+Left)"));
    forward_btn.set_tooltip_text(Some("Go forward (Alt+Right)"));
    up_btn.set_tooltip_text(Some("Go to parent directory"));
    home_btn.set_tooltip_text(Some("Go to home directory"));
    new_window_btn.set_tooltip_text(Some("Open a new window"));
    bookmark_btn.set_tooltip_text(Some("Bookmark current directory"));
    new_folder_btn.set_tooltip_text(Some("Create new folder"));
    new_file_btn.set_tooltip_text(Some("Create new file"));
    rename_btn.set_tooltip_text(Some("Rename selected item (F2)"));
    copy_btn.set_tooltip_text(Some("Copy selected items (Ctrl+C)"));
    move_btn.set_tooltip_text(Some("Move selected items (Ctrl+X)"));
    paste_btn.set_tooltip_text(Some("Paste items (Ctrl+V)"));
    trash_btn.set_tooltip_text(Some("Move to trash (Delete)"));
    open_trash_btn.set_tooltip_text(Some("View trash contents"));
    settings_btn.set_tooltip_text(Some("Open settings"));
    preview_toggle.set_tooltip_text(Some("Toggle file preview panel"));
    split_toggle.set_tooltip_text(Some("Toggle split view"));
    tree_toggle.set_tooltip_text(Some("Toggle tree view sidebar"));
    hidden_toggle.set_tooltip_text(Some("Show hidden files (Ctrl+H)"));
    help_btn.set_tooltip_text(Some("Keyboard shortcuts help"));

    let sidebar_list = ListBox::new();
    sidebar_list.set_selection_mode(SelectionMode::Single);

    let sidebar_stack = gtk::Stack::new();

    let sidebar_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    sidebar_scrolled.set_child(Some(&sidebar_list));
    sidebar_scrolled.set_width_request(190);
    sidebar_scrolled.set_vexpand(true);

    let (tree_list, tree_state) = ui::tree_view::build(locations::home_dir());

    let tree_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    tree_scrolled.set_child(Some(&tree_list));
    tree_scrolled.set_width_request(190);
    tree_scrolled.set_vexpand(true);

    sidebar_stack.add_named(&sidebar_scrolled, Some("places"));
    sidebar_stack.add_named(&tree_scrolled, Some("tree"));
    sidebar_stack.set_visible_child_name("places");

    let notebook = Notebook::new();
    notebook.set_show_tabs(true);
    notebook.set_show_border(false);
    notebook.set_vexpand(true);
    notebook.set_hexpand(true);

    let content = GtkBox::new(Orientation::Horizontal, 6);
    content.append(&sidebar_stack);
    content.append(&notebook);
    content.set_vexpand(true);

    let (preview_scrolled, preview_box) = ui::preview::build();
    content.append(&preview_scrolled);

    let split_pane = ui::split_pane::build(locations::home_dir());
    split_pane.container.set_visible(false);
    content.append(&split_pane.container.clone());

    let split_state = split_pane.state.clone();
    let split_store = split_pane.store.clone();
    let split_label = split_pane.location_label.clone();

    let status_bar = GtkBox::new(Orientation::Horizontal, 6);
    let status_label = Label::new(Some("Ready"));
    status_label.set_halign(gtk::Align::Start);
    status_label.set_hexpand(true);
    let selection_label = Label::new(Some(""));
    selection_label.set_halign(gtk::Align::End);

    let job_queue: JobQueue = Rc::new(RefCell::new(JobQueueState {
        pending: VecDeque::new(),
        running: false,
    }));

    status_bar.append(&status_label);
    status_bar.append(&selection_label);

    root.append(&toolbar1);
    root.append(&toolbar2);
    root.append(&content);
    root.append(&status_bar);

    location_entry.set_data("path-crumbs", path_crumbs.clone());
    location_entry.set_data("location-stack", location_stack.clone());
    location_entry.set_data("status-label", status_label.clone());
    location_entry.set_data("selection-label", selection_label.clone());
    location_entry.set_data("job-queue", job_queue.clone());
    location_entry.set_data("preview-panel", preview_scrolled.clone());
    location_entry.set_data("split-state", split_state.clone());
    location_entry.set_data("split-store", split_store.clone());
    location_entry.set_data("split-label", split_label.clone());
    location_entry.set_data("split-container", split_pane.container.clone());
    location_entry.set_data("preview-box", preview_box.clone());
    location_entry.set_data("main-window", window.clone());

    window.set_child(Some(&root));

    // Open directories passed on the command line.
    // Files open their parent directory.
    let mut initial_dirs: Vec<PathBuf> = Vec::new();

    for arg in initial_args {
        let path = normalize(PathBuf::from(arg));

        if path.is_dir() {
            initial_dirs.push(path);
        } else if path.is_file() {
            if let Some(parent) = path.parent() {
                initial_dirs.push(parent.to_path_buf());
            }
        }
    }

    if initial_dirs.is_empty() {
        initial_dirs.push(locations::home_dir());
    }

    for dir in initial_dirs {
        add_tab(
            &notebook,
            &ctx,
            dir,
            &window,
            &location_entry,
            &search_entry,
            &hidden_toggle,
            &sidebar_list,
            &watcher_manager,
        );
    }

    // Poll for config changes
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(shared_config) = config_rx.recv().await {
                // Apply theme if changed
                let theme_mode = crate::ui::theme::ThemeMode::from_str(&shared_config.theme_mode);
                if let Some(display) = gdk::Display::default() {
                    crate::ui::theme::apply_theme(&display, theme_mode);
                }

                // Refresh current tab
                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    tab_state.borrow_mut().show_hidden = shared_config.show_hidden_files;

                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
    }

    // --- Inotify Receiver ---
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Ok(()) = receiver.recv().await {
                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    refresh_tab(
                        &tab_state,
                        &store,
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

    // --- Keyboard Shortcuts ---
    {
        let key_controller = gtk::EventControllerKey::new();
        window.add_controller(key_controller.clone());

        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let location_stack = location_stack.clone();

        key_controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);

            if ctrl && key == gtk::gdk::Key::l {
                location_stack.set_visible_child_name("entry");
                location_entry.grab_focus();
                return glib::Propagation::Stop;
            }

            if let Some(focus) = window.focus() {
                if focus.downcast_ref::<gtk::Entry>().is_some()
                    || focus.downcast_ref::<gtk::SearchEntry>().is_some()
                {
                    return glib::Propagation::Proceed;
                }
            }

            let alt = modifier.contains(gtk::gdk::ModifierType::ALT_MASK);
            let active = get_active_widgets(&notebook);

            // Type-ahead: typing letters jumps to matching files
            if !ctrl && !alt {
                if let Some(ch) = key.to_unicode() {
                    if ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                        if let Some((_, grid, store, selection)) = &active {
                            typeahead_select(ch, grid, store, selection);
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }

            match key {
                k if ctrl && k == gtk::gdk::Key::c => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if !selected.is_empty() {
                            let paths: Vec<PathBuf> =
                                selected.iter().map(|item| item.get_path()).collect();
                            ctx.borrow_mut().pending = Some((PendingOp::Copy, paths));
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::x => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if !selected.is_empty() {
                            let paths: Vec<PathBuf> =
                                selected.iter().map(|item| item.get_path()).collect();
                            ctx.borrow_mut().pending = Some((PendingOp::Move, paths));
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::v => {
                    let pending = ctx.borrow_mut().pending.take();
                    if let Some((operation, sources)) = pending {
                        if let Some((tab_state, _, _, _)) = &active {
                            let destination_dir = tab_state.borrow().current.clone();
                            start_paste_job_ui(
                                &window,
                                &notebook,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                                &watcher_manager,
                                operation,
                                sources,
                                destination_dir,
                            );
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::t => {
                    if let Some((tab_state, _, _, _)) = &active {
                        let current = tab_state.borrow().current.clone();
                        add_tab(
                            &notebook,
                            &ctx,
                            current,
                            &window,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                            &watcher_manager,
                        );
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::w => {
                    if let Some(page_num) = notebook.current_page() {
                        notebook.remove_page(page_num);
                        update_watcher(&notebook, &watcher_manager);
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::h => {
                    if let Some((tab_state, _, store, _)) = &active {
                        let mut s = tab_state.borrow_mut();
                        s.show_hidden = !s.show_hidden;
                        drop(s);
                        refresh_tab(
                            tab_state,
                            store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                    return glib::Propagation::Stop;
                }
                k if alt && k == gtk::gdk::Key::Left => {
                    if let Some((tab_state, _, store, _)) = &active {
                        let current = tab_state.borrow().current.clone();
                        let previous = tab_state.borrow_mut().history.go_back(&current);
                        if let Some(prev) = previous {
                            tab_state.borrow_mut().current = prev;
                            refresh_tab(
                                tab_state,
                                store,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                            );
                            update_watcher(&notebook, &watcher_manager);
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if alt && k == gtk::gdk::Key::Right => {
                    if let Some((tab_state, _, store, _)) = &active {
                        let current = tab_state.borrow().current.clone();
                        let next = tab_state.borrow_mut().history.go_forward(&current);
                        if let Some(n) = next {
                            tab_state.borrow_mut().current = n;
                            refresh_tab(
                                tab_state,
                                store,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                            );
                            update_watcher(&notebook, &watcher_manager);
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if key == gtk::gdk::Key::Delete => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if !selected.is_empty() {
                            let paths: Vec<PathBuf> =
                                selected.iter().map(|item| item.get_path()).collect();
                            start_trash_job_ui(
                                &window,
                                &notebook,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                                &watcher_manager,
                                paths,
                            );
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if key == gtk::gdk::Key::F2 => {
                    if let Some((tab_state, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if selected.len() == 1 {
                            let item = selected[0].clone();
                            let source = item.get_path();
                            let initial_name = item.name();
                            let window_clone = window.clone();
                            let notebook_clone = notebook.clone();
                            let ctx_clone = ctx.clone();
                            let location_entry_clone = location_entry.clone();
                            let search_entry_clone = search_entry.clone();
                            let hidden_toggle_clone = hidden_toggle.clone();
                            let sidebar_list_clone = sidebar_list.clone();
                            let watcher_manager_clone = watcher_manager.clone();

                            dialogs::show_text_dialog(
                                &window_clone,
                                "Rename",
                                &initial_name,
                                "Rename",
                                move |name| {
                                    if name.is_empty() {
                                        return;
                                    }
                                    if let Err(err) =
                                        operations::rename::rename_path(&source, &name)
                                    {
                                        dialogs::show_error(
                                            &window_clone,
                                            &format!("Could not rename: {err}"),
                                        );
                                    }
                                    if let Some((tab_state, _, store, _)) =
                                        get_active_widgets(&notebook_clone)
                                    {
                                        refresh_tab(
                                            &tab_state,
                                            &store,
                                            &ctx_clone,
                                            &location_entry_clone,
                                            &search_entry_clone,
                                            &hidden_toggle_clone,
                                            &sidebar_list_clone,
                                        );
                                        update_watcher(&notebook_clone, &watcher_manager_clone);
                                    }
                                },
                            );
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if key == gtk::gdk::Key::F5 => {
                    if let Some((tab_state, _, store, _)) = &active {
                        refresh_tab(
                            tab_state,
                            store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                    return glib::Propagation::Stop;
                }
                _ => {}
            }

            glib::Propagation::Proceed
        });
    }

    // --- Global Signals ---
    {
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        notebook.connect_switch_page(move |nb, _, _| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(nb) {
                refresh_tab(
                    &tab_state,
                    &store,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                );
                update_watcher(nb, &watcher_manager);
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry_clone = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let search_recursive_toggle = search_recursive_toggle.clone();

        search_entry.connect_activate(move |entry| {
            let query = entry.text().to_string();

            if query.is_empty() {
                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    tab_state.borrow_mut().search_query = String::new();
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry_clone,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
                return;
            }

            let recursive = search_recursive_toggle.is_active();

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                let root = tab_state.borrow().current.clone();

                if recursive {
                    let filters = search::filters::SearchFilters {
                        query: query.clone(),
                        recursive: true,
                        match_file_name: true,
                        match_content: false,
                        file_types: Vec::new(),
                        min_size_bytes: None,
                        max_size_bytes: None,
                    };

                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let (tx, rx) = std::sync::mpsc::channel();

                    search::engine::start_search(root.clone(), filters, cancel.clone(), tx);

                    let tab_state = tab_state.clone();
                    let store = store.clone();

                    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));

                    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                        let rx = rx.lock().unwrap();

                        if let Ok(results) = rx.try_recv() {
                            let items: Vec<crate::filesystem::directory::Item> =
                                results.into_iter().map(|r| r.item).collect();

                            let mut s = tab_state.borrow_mut();
                            s.search_query = query.clone();
                            s.items = items.clone();
                            drop(s);

                            crate::ui::grid_view::render(&store, &items);

                            return glib::ControlFlow::Break;
                        }

                        glib::ControlFlow::Continue
                    });
                } else {
                    tab_state.borrow_mut().search_query = query.clone();
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry_clone,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry_clone = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        location_entry.connect_activate(move |entry| {
            if let Some(stack) = entry.data::<gtk::Stack>("location-stack") {
                stack.set_visible_child_name("crumbs");
            }

            let text = entry.text().to_string();
            let path = PathBuf::from(text);

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                if path.is_dir() {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry_clone,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        sidebar_list.connect_row_activated(move |_, row| {
            if let Some(path) = sidebar::resolve_click(row) {
                // Recent entries are files → open them instead of navigating
                if path.is_file() {
                    open_file_default(&path);
                    return;
                }

                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
    }

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

    // Tree View Toggle
    {
        let sidebar_stack = sidebar_stack.clone();
        tree_toggle.connect_toggled(move |toggle| {
            if toggle.is_active() {
                sidebar_stack.set_visible_child_name("tree");
            } else {
                sidebar_stack.set_visible_child_name("places");
            }
        });
    }

    // Tree View Row Activation
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let tree_list = tree_list.clone();
        let tree_state = tree_state.clone();

        tree_list.connect_row_activated(move |_, row| {
            if let Some(path_str) = row.name() {
                let path = PathBuf::from(path_str);

                // Toggle folder expansion in the tree
                ui::tree_view::toggle_folder(&tree_state, &tree_list, path.clone());

                // Navigate the active tab to the clicked folder
                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
    }

    // New Window Button
    {
        let app = app.clone();
        new_window_btn.connect_clicked(move |_| {
            let window = ui::window::create_window(&app, "MITOS Files");
            let label = gtk::Label::new(Some(
                "New window opened.\nFull multi-window file manager\nrequires duplicating the tab/notebook setup.\nThis is a placeholder window.",
            ));
            label.set_wrap(true);
            label.set_justify(gtk::Justification::Center);
            window.set_child(Some(&label));
            window.present();
        });
    }

    // Help Button
    {
        let window = window.clone();
        help_btn.connect_clicked(move |_| {
            let shortcuts = ui::accessibility::setup_keyboard_help();

            let dialog = gtk::Dialog::builder()
                .title("Keyboard Shortcuts")
                .transient_for(&window)
                .modal(true)
                .build();

            dialog.add_button("Close", gtk::ResponseType::Close);

            let content = dialog.content_area();
            content.set_margin_top(12);
            content.set_margin_bottom(12);
            content.set_margin_start(12);
            content.set_margin_end(12);

            let grid = gtk::Grid::new();
            grid.set_column_spacing(16);
            grid.set_row_spacing(8);

            let mut row = 0;
            for (key, description) in &shortcuts {
                let key_label = gtk::Label::new(Some(key));
                key_label.set_halign(gtk::Align::End);
                key_label.add_css_class("heading");

                let desc_label = gtk::Label::new(Some(description));
                desc_label.set_halign(gtk::Align::Start);

                grid.attach(&key_label, 0, row, 1, 1);
                grid.attach(&desc_label, 1, row, 1, 1);
                row += 1;
            }

            content.append(&grid);
            dialog.connect_response(|dialog, _| dialog.close());
            dialog.present();
        });
    }

    // Toolbar Buttons
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        back_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                let current = tab_state.borrow().current.clone();
                let previous = tab_state.borrow_mut().history.go_back(&current);
                if let Some(prev) = previous {
                    tab_state.borrow_mut().current = prev;
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        forward_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                let current = tab_state.borrow().current.clone();
                let next = tab_state.borrow_mut().history.go_forward(&current);
                if let Some(n) = next {
                    tab_state.borrow_mut().current = n;
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        up_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                let current = tab_state.borrow().current.clone();
                if let Some(parent) = current.parent() {
                    navigate_to(&tab_state, parent.to_path_buf());
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        home_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                navigate_to(&tab_state, locations::home_dir());
                refresh_tab(
                    &tab_state,
                    &store,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                );
                update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        hidden_toggle.connect_toggled(move |toggle| {
            let is_active = toggle.is_active();
            config::settings::set_show_hidden(is_active);

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                if tab_state.borrow().show_hidden != is_active {
                    tab_state.borrow_mut().show_hidden = is_active;
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                    update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        new_folder_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                dialogs::show_text_dialog(&window_parent, "New Folder", "New Folder", "Create", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let store = store.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();
                    let watcher_manager = watcher_manager.clone();
                    let notebook = notebook.clone();

                    move |name| {
                        if name.is_empty() {
                            return;
                        }
                        let parent = tab_state.borrow().current.clone();
                        if let Err(err) = operations::create::create_folder(&parent, &name) {
                            dialogs::show_error(
                                &window_error,
                                &format!("Could not create folder: {err}"),
                            );
                        }
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        new_file_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                dialogs::show_text_dialog(&window_parent, "New File", "new-file.txt", "Create", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let store = store.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();
                    let watcher_manager = watcher_manager.clone();
                    let notebook = notebook.clone();

                    move |name| {
                        if name.is_empty() {
                            return;
                        }
                        let parent = tab_state.borrow().current.clone();
                        if let Err(err) = operations::create::create_file(&parent, &name) {
                            dialogs::show_error(
                                &window_error,
                                &format!("Could not create file: {err}"),
                            );
                        }
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
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
        let watcher_manager = watcher_manager.clone();

        rename_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, store, selection)) = get_active_widgets(&notebook) {
                let selected = grid_view::selected_items(&selection, &store);
                if selected.len() != 1 {
                    return;
                }

                let item = selected[0].clone();
                let source = item.get_path();
                let initial_name = item.name();

                dialogs::show_text_dialog(&window_parent, "Rename", &initial_name, "Rename", {
                    let window_error = window_parent.clone();
                    let tab_state = tab_state.clone();
                    let store = store.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list.clone();
                    let watcher_manager = watcher_manager.clone();
                    let notebook = notebook.clone();

                    move |name| {
                        if name.is_empty() {
                            return;
                        }
                        if let Err(err) = operations::rename::rename_path(&source, &name) {
                            dialogs::show_error(&window_error, &format!("Could not rename: {err}"));
                        }
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                });
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();

        copy_btn.connect_clicked(move |_| {
            if let Some((_, _, store, selection)) = get_active_widgets(&notebook) {
                let selected = grid_view::selected_items(&selection, &store);
                if selected.is_empty() {
                    return;
                }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.get_path()).collect();
                ctx.borrow_mut().pending = Some((PendingOp::Copy, paths));
            }
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();

        move_btn.connect_clicked(move |_| {
            if let Some((_, _, store, selection)) = get_active_widgets(&notebook) {
                let selected = grid_view::selected_items(&selection, &store);
                if selected.is_empty() {
                    return;
                }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.get_path()).collect();
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
        let watcher_manager = watcher_manager.clone();

        paste_btn.connect_clicked(move |_| {
            let pending = ctx.borrow_mut().pending.take();
            let Some((operation, sources)) = pending else {
                return;
            };

            if let Some((tab_state, _, _, _)) = get_active_widgets(&notebook) {
                let destination_dir = tab_state.borrow().current.clone();
                start_paste_job_ui(
                    &window_error,
                    &notebook,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    operation,
                    sources,
                    destination_dir,
                );
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
        let watcher_manager = watcher_manager.clone();

        trash_btn.connect_clicked(move |_| {
            if let Some((_, _, store, selection)) = get_active_widgets(&notebook) {
                let selected = grid_view::selected_items(&selection, &store);
                if selected.is_empty() {
                    return;
                }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.get_path()).collect();
                start_trash_job_ui(
                    &window_error,
                    &notebook,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    paths,
                );
            }
        });
    }

    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        open_trash_btn.connect_clicked(move |_| {
            let refresh_main: Rc<dyn Fn()> = Rc::new({
                let notebook = notebook.clone();
                let ctx = ctx.clone();
                let location_entry = location_entry.clone();
                let search_entry = search_entry.clone();
                let hidden_toggle = hidden_toggle.clone();
                let sidebar_list = sidebar_list.clone();
                let watcher_manager = watcher_manager.clone();

                move || {
                    if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                }
            });

            ui::trash_view::show(&window, refresh_main);
        });
    }

    {
        let location_entry = location_entry.clone();
        preview_toggle.connect_toggled(move |toggle| {
            if let Some(panel) = location_entry.data::<gtk::ScrolledWindow>("preview-panel") {
                panel.set_visible(toggle.is_active());
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
        let watcher_manager = watcher_manager.clone();

        list_toggle.connect_toggled(move |toggle| {
            VIEW_MODE_LIST.store(toggle.is_active(), Ordering::Relaxed);

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                refresh_tab(
                    &tab_state,
                    &store,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                );
                update_watcher(&notebook, &watcher_manager);
            }
        });
    }

    {
        let location_entry = location_entry.clone();
        split_toggle.connect_toggled(move |toggle| {
            if let Some(container) = location_entry.data::<gtk::Box>("split-container") {
                container.set_visible(toggle.is_active());
            }
        });
    }

    {
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        settings_btn.connect_clicked(move |_| {
            let apply_changes: Rc<dyn Fn()> = Rc::new({
                let notebook = notebook.clone();
                let ctx = ctx.clone();
                let location_entry = location_entry.clone();
                let search_entry = search_entry.clone();
                let hidden_toggle = hidden_toggle.clone();
                let sidebar_list = sidebar_list.clone();
                let watcher_manager = watcher_manager.clone();

                move || {
                    if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                        let show_hidden = config::settings::show_hidden_default();
                        tab_state.borrow_mut().show_hidden = show_hidden;
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }

                    let theme_mode = config::settings::theme_mode();
                    if let Some(display) = gdk::Display::default() {
                        ui::theme::apply_theme(&display, theme_mode);
                    }
                }
            });

            ui::settings::show(&window, apply_changes);
        });
    }

    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let location_entry = location_entry.clone();

        bookmark_btn.connect_clicked(move |_| {
            if let Some((tab_state, _, _, _)) = get_active_widgets(&notebook) {
                let mut c = ctx.borrow_mut();
                let current = tab_state.borrow().current.clone();
                let name = current
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| current.to_string_lossy().to_string());

                if c.bookmarks.iter().any(|b| b.path == current) {
                    return;
                }

                bookmarks::add(&mut c.bookmarks, name, current);
                drop(c);
                if let Some(win) = location_entry.data::<gtk::ApplicationWindow>("main-window") {
                    sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
                }
                update_watcher(&notebook, &watcher_manager);
            }
        });
    }

    // --- Volume Monitor (USB / Network Hotplug) ---
    {
        let monitor = gio::VolumeMonitor::get();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        let rebuild_and_check = move |unmounted_path: Option<PathBuf>| {
            if let Some(win) = location_entry.data::<gtk::ApplicationWindow>("main-window") {
                sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
            }

            if let Some(lost_path) = unmounted_path {
                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    let current = tab_state.borrow().current.clone();
                    if current.starts_with(&lost_path) || current == lost_path {
                        tab_state.borrow_mut().current = locations::home_dir();
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                }
            }
        };

        let rebuild_add = rebuild_and_check.clone();
        monitor.connect_mount_added(move |_, _| rebuild_add(None));

        let rebuild_remove = rebuild_and_check.clone();
        monitor.connect_mount_removed(move |_, mount| {
            let path = mount.root().path();
            rebuild_remove(path);
        });

        let rebuild_vol_add = rebuild_and_check.clone();
        monitor.connect_volume_added(move |_, _| rebuild_vol_add(None));

        let rebuild_vol_rem = rebuild_and_check.clone();
        monitor.connect_volume_removed(move |_, _| rebuild_vol_rem(None));
    }

    // Portal Receiver
    {
        let window = window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            while let Ok(request) = portal_rx.try_recv() {
                match request {
                    portal::service::PortalRequest::OpenFile { title, response_tx } => {
                        let dialog = gtk::FileDialog::builder().title(&title).modal(true).build();
                        let response_tx = response_tx.clone();
                        dialog.open(Some(&window), None, move |result| match result {
                            Ok(file) => {
                                let path = file
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                let _ = response_tx
                                    .send(portal::service::PortalResponse::Selected(vec![path]));
                            }
                            Err(_) => {
                                let _ =
                                    response_tx.send(portal::service::PortalResponse::Cancelled);
                            }
                        });
                    }
                    portal::service::PortalRequest::SaveFile {
                        title,
                        default_name,
                        response_tx,
                    } => {
                        let dialog = gtk::FileDialog::builder()
                            .title(&title)
                            .initial_name(&default_name)
                            .modal(true)
                            .build();
                        let response_tx = response_tx.clone();
                        dialog.save(Some(&window), None, move |result| match result {
                            Ok(file) => {
                                let path = file
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                let _ = response_tx
                                    .send(portal::service::PortalResponse::Selected(vec![path]));
                            }
                            Err(_) => {
                                let _ =
                                    response_tx.send(portal::service::PortalResponse::Cancelled);
                            }
                        });
                    }
                    portal::service::PortalRequest::OpenFolder { title, response_tx } => {
                        let dialog = gtk::FileDialog::builder().title(&title).modal(true).build();
                        let response_tx = response_tx.clone();
                        dialog.select_folder(Some(&window), None, move |result| match result {
                            Ok(file) => {
                                let path = file
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                let _ = response_tx
                                    .send(portal::service::PortalResponse::Selected(vec![path]));
                            }
                            Err(_) => {
                                let _ =
                                    response_tx.send(portal::service::PortalResponse::Cancelled);
                            }
                        });
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

fn show_context_menu<W: IsA<gtk::Widget>>(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    parent: &W,
    store: &gio::ListStore,
    selection: &gtk::MultiSelection,
    location_entry: &Entry,
    search_entry: &SearchEntry,
    hidden_toggle: &CheckButton,
    sidebar_list: &ListBox,
    watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    items: Vec<ItemObject>,
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
    popover.set_parent(parent);
    popover.set_pointing_to(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let open_btn = Button::with_label("Open");
    let open_tab_btn = Button::with_label("Open in New Tab");
    let open_with_btn = Button::with_label("Open With");
    let compress_btn = Button::with_label("Compress to ZIP");
    let extract_btn = Button::with_label("Extract Here");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let copy_to_split_btn = Button::with_label("Copy to Split Pane");
    let rename_btn = Button::with_label("Rename");
    let batch_rename_btn = Button::with_label("Batch Rename");
    let trash_btn = Button::with_label("Trash");
    let properties_btn = Button::with_label("Properties");

    open_btn.set_sensitive(count == 1);
    open_tab_btn.set_sensitive(count == 1 && single_item.as_ref().map_or(false, |i| i.is_dir()));
    open_with_btn.set_sensitive(count == 1 && single_item.as_ref().map_or(false, |i| !i.is_dir()));
    copy_to_split_btn.set_sensitive(count >= 1);
    compress_btn.set_sensitive(!items.is_empty());
    extract_btn.set_sensitive(
        count == 1
            && single_item.as_ref().map_or(false, |item| {
                operations::archive::is_supported_archive(&item.get_path())
            }),
    );
    rename_btn.set_sensitive(count == 1);
    batch_rename_btn.set_sensitive(count >= 2);
    properties_btn.set_sensitive(count == 1);

    menu_box.append(&open_btn);
    menu_box.append(&open_tab_btn);
    menu_box.append(&open_with_btn);
    menu_box.append(&compress_btn);
    menu_box.append(&extract_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&move_btn);
    menu_box.append(&copy_to_split_btn);
    menu_box.append(&rename_btn);
    menu_box.append(&batch_rename_btn);
    menu_box.append(&trash_btn);
    menu_box.append(&properties_btn);

    // Plugin Actions
    let loaded_plugins = plugins::loader::load_plugins();
    for (_plugin_path, manifest) in &loaded_plugins {
        for action in &manifest.actions {
            let applies = match action.applies_to {
                plugins::manifest::AppliesTo::Files => items.iter().all(|i| !i.is_dir()),
                plugins::manifest::AppliesTo::Folders => items.iter().all(|i| i.is_dir()),
                plugins::manifest::AppliesTo::Both => true,
            };

            if !applies {
                continue;
            }

            let btn = Button::with_label(&action.label);
            btn.set_has_frame(false);

            let command = action.command.clone();
            let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();
            let popover_clone = popover.clone();
            let window_clone = window.clone();

            btn.connect_clicked(move |_| {
                popover_clone.popdown();
                if let Err(err) = plugins::loader::execute_plugin_action(&command, &paths) {
                    crate::ui::dialogs::show_error(&window_clone, &err);
                }
            });

            menu_box.append(&btn);
        }
    }

    popover.set_child(Some(&menu_box));

    {
        let popover = popover.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let item = single_item.clone();
        let window = window.clone();

        open_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = item.clone() else {
                return;
            };
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                if item.is_dir() {
                    navigate_to(&tab_state, item.get_path());
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                    );
                } else {
                    let path = item.get_path();
                    let mime = item.mime_type();
                    if let Some(app) = crate::mime::applications::default_app_for_mime(&mime) {
                        if let Err(err) =
                            crate::mime::applications::launch_app_with_file(&app, &path)
                        {
                            crate::ui::dialogs::show_error(
                                &window,
                                &format!("Failed to open: {}", err),
                            );
                        }
                    } else {
                        let _ = Command::new("xdg-open").arg(&path).spawn();
                    }
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
        let watcher_manager = watcher_manager.clone();

        open_tab_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = single_item.clone() else {
                return;
            };
            if item.is_dir() {
                add_tab(
                    &notebook,
                    &ctx,
                    item.get_path(),
                    &window,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                );
            }
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let single_item = single_item.clone();

        open_with_btn.connect_clicked(move |_| {
            let Some(item) = single_item.clone() else {
                return;
            };
            let path = item.get_path();
            let mime = item.mime_type();
            let apps = crate::mime::applications::apps_for_mime(&mime);
            let display_apps = crate::mime::applications::app_display_names(&apps);

            if display_apps.is_empty() {
                crate::ui::dialogs::show_error(
                    &window,
                    &format!("No applications found for MIME type: {}", mime),
                );
                return;
            }

            let sub_popover = gtk::Popover::new();
            sub_popover.set_has_arrow(true);
            sub_popover.set_autohide(true);
            sub_popover.set_parent(&open_with_btn);

            let sub_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
            sub_box.set_margin_top(4);
            sub_box.set_margin_bottom(4);
            sub_box.set_margin_start(4);
            sub_box.set_margin_end(4);

            for (name, app_info) in &display_apps {
                let btn = gtk::Button::with_label(name);
                btn.set_has_frame(false);
                btn.set_halign(gtk::Align::Fill);

                let app_info = app_info.clone();
                let path = path.clone();
                let sub_popover = sub_popover.clone();
                let window = window.clone();

                btn.connect_clicked(move |_| {
                    sub_popover.popdown();
                    if let Err(err) =
                        crate::mime::applications::launch_app_with_file(&app_info, &path)
                    {
                        crate::ui::dialogs::show_error(
                            &window,
                            &format!("Failed to open: {}", err),
                        );
                    }
                });
                sub_box.append(&btn);
            }

            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sub_box.append(&sep);

            let default_btn = gtk::Button::with_label("Set Default App...");
            default_btn.set_has_frame(false);

            let display_apps_clone = display_apps.clone();
            let mime_clone = mime.clone();
            let window_clone = window.clone();

            default_btn.connect_clicked(move |_| {
                sub_popover.popdown();
                show_default_app_picker(&window_clone, display_apps_clone.clone(), &mime_clone);
            });

            sub_box.append(&default_btn);
            sub_popover.set_child(Some(&sub_box));
            sub_popover.popup();
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
        let watcher_manager = watcher_manager.clone();
        let sources: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        compress_btn.connect_clicked(move |_| {
            popover.popdown();
            if let Some((tab_state, _, _, _)) = get_active_widgets(&notebook) {
                let destination_dir = tab_state.borrow().current.clone();
                start_compress_zip_job_ui(
                    &window,
                    &notebook,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    sources.clone(),
                    destination_dir,
                );
            }
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
        let watcher_manager = watcher_manager.clone();
        let archive_item = single_item.clone();

        extract_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = archive_item.clone() else {
                return;
            };
            if !operations::archive::is_supported_archive(&item.get_path()) {
                return;
            }
            if let Some((tab_state, _, _, _)) = get_active_widgets(&notebook) {
                let destination_dir = tab_state.borrow().current.clone();
                start_extract_archive_job_ui(
                    &window,
                    &notebook,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                    &watcher_manager,
                    item.get_path(),
                    destination_dir,
                );
            }
        });
    }

    {
        let popover = popover.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        copy_btn.connect_clicked(move |_| {
            popover.popdown();
            ctx.borrow_mut().pending = Some((PendingOp::Copy, paths.clone()));
        });
    }

    {
        let popover = popover.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        move_btn.connect_clicked(move |_| {
            popover.popdown();
            ctx.borrow_mut().pending = Some((PendingOp::Move, paths.clone()));
        });
    }

    {
        let popover = popover.clone();
        let location_entry = location_entry.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        copy_to_split_btn.connect_clicked(move |_| {
            popover.popdown();
            if let Some(split_state) = location_entry.data::<std::rc::Rc<
                std::cell::RefCell<ui::split_pane::SplitPaneState>,
            >>("split-state")
            {
                let dest = split_state.borrow().current.clone();
                for source in &paths {
                    let file_name = source.file_name().unwrap_or_default();
                    let target = dest.join(file_name);
                    let _ = crate::operations::copy::copy_path(source, &target);
                }

                if let Some(store) = location_entry.data::<gio::ListStore>("split-store") {
                    if let Some(label) = location_entry.data::<Label>("split-label") {
                        ui::split_pane::refresh_pane(&split_state, &store, &label);
                    }
                }
            }
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
        let watcher_manager = watcher_manager.clone();
        let single_item = single_item.clone();

        rename_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = single_item.clone() else {
                return;
            };
            let source = item.get_path();
            let initial_name = item.name();

            dialogs::show_text_dialog(&window, "Rename", &initial_name, "Rename", {
                let window = window.clone();
                let notebook = notebook.clone();
                let ctx = ctx.clone();
                let location_entry = location_entry.clone();
                let search_entry = search_entry.clone();
                let hidden_toggle = hidden_toggle.clone();
                let sidebar_list = sidebar_list.clone();
                let watcher_manager = watcher_manager.clone();

                move |name| {
                    if name.is_empty() {
                        return;
                    }
                    if let Err(err) = operations::rename::rename_path(&source, &name) {
                        dialogs::show_error(&window, &format!("Could not rename: {err}"));
                    }
                    if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                        refresh_tab(
                            &tab_state,
                            &store,
                            &ctx,
                            &location_entry,
                            &search_entry,
                            &hidden_toggle,
                            &sidebar_list,
                        );
                        update_watcher(&notebook, &watcher_manager);
                    }
                }
            });
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let notebook = notebook.clone(); // FIXED TYPO: notebooki -> notebook
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();

        let items_for_rename: Vec<(String, PathBuf)> = items
            .iter()
            .map(|item| (item.name(), item.get_path()))
            .collect();

        batch_rename_btn.connect_clicked(move |_| {
            popover.popdown();

            let window_clone = window.clone();
            let notebook_clone = notebook.clone();
            let ctx_clone = ctx.clone();
            let location_entry_clone = location_entry.clone();
            let search_entry_clone = search_entry.clone();
            let hidden_toggle_clone = hidden_toggle.clone();
            let sidebar_list_clone = sidebar_list.clone();
            let watcher_manager_clone = watcher_manager.clone();

            ui::batch_rename::show(&window, items_for_rename.clone(), move |renames| {
                start_batch_rename_job_ui(
                    &window_clone,
                    &notebook_clone,
                    &ctx_clone,
                    &location_entry_clone,
                    &search_entry_clone,
                    &hidden_toggle_clone,
                    &sidebar_list_clone,
                    &watcher_manager_clone,
                    renames,
                );
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
        let watcher_manager = watcher_manager.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        trash_btn.connect_clicked(move |_| {
            popover.popdown();
            start_trash_job_ui(
                &window,
                &notebook,
                &ctx,
                &location_entry,
                &search_entry,
                &hidden_toggle,
                &sidebar_list,
                &watcher_manager,
                paths.clone(),
            );
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let item = single_item.clone();

        properties_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = item.clone() else { return };
            ui::properties::show(&window, &item);
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
            if let Some(win) = location_entry.data::<gtk::ApplicationWindow>("main-window") {
                sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
            }
            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                refresh_tab(
                    &tab_state,
                    &store,
                    &ctx,
                    &location_entry,
                    &search_entry,
                    &hidden_toggle,
                    &sidebar_list,
                );
            }
        });
    }

    popover.popup();
}

fn show_default_app_picker(
    parent: &ApplicationWindow,
    apps: Vec<(String, gtk::gio::AppInfo)>,
    mime: &str,
) {
    let window = gtk::Window::builder()
        .title("Set Default Application")
        .transient_for(parent)
        .default_width(400)
        .default_height(350)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let label = gtk::Label::new(Some(&format!("MIME type: {}", mime)));
    label.set_halign(gtk::Align::Start);

    let list_box = gtk::ListBox::new();
    list_box.set_selection_mode(gtk::SelectionMode::Single);

    for (name, _) in &apps {
        let row = gtk::ListBoxRow::new();
        let row_label = gtk::Label::new(Some(name));
        row_label.set_halign(gtk::Align::Start);
        row.set_child(Some(&row_label));
        list_box.append(&row);
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&list_box));
    scrolled.set_vexpand(true);

    let button_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let cancel_btn = gtk::Button::with_label("Cancel");
    let ok_btn = gtk::Button::with_label("Set as Default");
    button_box.append(&cancel_btn);
    button_box.append(&ok_btn);

    root.append(&label);
    root.append(&scrolled);
    root.append(&button_box);
    window.set_child(Some(&root));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let window = window.clone();
        let list_box = list_box.clone();
        let apps = apps.clone();
        let mime = mime.to_string();

        ok_btn.connect_clicked(move |_| {
            if let Some(row) = list_box.selected_row() {
                let index = row.index();
                if index >= 0 && (index as usize) < apps.len() {
                    let (_, app_info) = &apps[index as usize];
                    if let Err(err) = crate::mime::applications::set_default_app(app_info, &mime) {
                        crate::ui::dialogs::show_error(
                            &window,
                            &format!("Failed to set default: {}", err),
                        );
                    }
                }
            }
            window.close();
        });
    }

    window.present();
}

fn send_job_notification(window: &ApplicationWindow, title: &str, body: &str) {
    if let Some(app) = window.application() {
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        notification.set_priority(gio::NotificationPriority::Normal);

        app.send_notification(Some("mitos-files-job"), &notification);
    }
}

fn filesystem_free_string(path: &std::path::Path) -> String {
    let file = gio::File::for_path(path);
    if let Ok(info) = file.query_filesystem_info(
        "filesystem::free",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        if let Some(free) = info.attribute_u64("filesystem::free") {
            return metadata::format_size(free);
        }
    }
    "?".to_string()
}

fn typeahead_select(
    ch: char,
    grid: &gtk::GridView,
    store: &gio::ListStore,
    selection: &gtk::MultiSelection,
) {
    TYPEAHEAD_BUFFER.with(|buf| {
        let mut b = buf.borrow_mut();

        if b.1.elapsed().as_millis() > 800 {
            b.0.clear();
        }

        b.0.push(ch);
        b.1 = std::time::Instant::now();

        let query = b.0.to_lowercase();

        for i in 0..store.n_items() {
            let Some(obj) = store.item(i) else { continue };
            let Some(item) = obj.downcast_ref::<ItemObject>() else {
                continue;
            };

            if item.name().to_lowercase().starts_with(&query) {
                selection.set_selected(i);
                grid.scroll_to(
                    i,
                    gtk::ListScrollFlags::SELECT | gtk::ListScrollFlags::FOCUS,
                    None,
                );
                break;
            }
        }
    });
}
