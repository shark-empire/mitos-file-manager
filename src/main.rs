mod app;
mod config;
mod desktop;
mod error;
mod filesystem;
mod mime;
mod navigation;
mod operations;
mod plugins;
mod portal;
mod search;
mod ui;
mod util;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::WidgetExt;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry, Label, ListBox,
    Notebook, Orientation, ScrolledWindow, SearchEntry, SelectionMode,
};

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
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
use util::{get_obj_data, set_obj_data};

static VIEW_MODE_LIST: AtomicBool = AtomicBool::new(false);

const NOTHING_TO_PASTE: &str =
    "Copy or cut some items first (Ctrl+C or Ctrl+X), then paste them here.";

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
    /// Permanent deletion -- no Trash, no undo.
    Delete {
        paths: Vec<PathBuf>,
    },
    CompressZip {
        sources: Vec<PathBuf>,
        archive_path: PathBuf,
    },
    CompressTarGz {
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

    // "Retry as administrator" re-runs this executable through pkexec in a
    // window-less helper mode: one operation, then exit -- no GTK, no D-Bus
    // services. See `operations::privileged`.
    if args.first().map(String::as_str) == Some(operations::privileged::FLAG) {
        std::process::exit(operations::privileged::run_helper(&args[1..]));
    }

    let app = Application::builder()
        .application_id("org.mitos.file-manager")
        .build();

    app.connect_activate(move |app| {
        // Everything here runs once per process, before the first
        // window: settings are loaded from disk (a second `build_ui`
        // call, e.g. from "New Window", reads the already-loaded values
        // instead of re-reading the file, so it reflects whatever's
        // currently live rather than resetting anything the user just
        // changed), and both D-Bus services claim a well-known bus name
        // each -- starting either twice would have the second attempt
        // fail to claim an already-owned name.
        config::settings::load();
        VIEW_MODE_LIST.store(
            config::settings::default_view() == "list",
            Ordering::Relaxed,
        );

        let portal_rx = portal::service::start();
        let desktop_rx = desktop::service::start();
        desktop::trash_service::start();

        build_ui(app, &args, Some(portal_rx), Some(desktop_rx));
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
    let widget = notebook.nth_page(Some(page_num))?;
    let state: Rc<RefCell<TabState>> = get_obj_data(&widget, "tab-state")?;
    let grid: gtk::GridView = get_obj_data(&widget, "grid-view")?;
    let store: gio::ListStore = get_obj_data(&widget, "list-store")?;
    let selection: gtk::MultiSelection = get_obj_data(&widget, "selection-model")?;
    Some((state, grid, store, selection))
}

fn normalize(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

/// Move a tab to `requested`, recording where it was so Back can return.
/// Fails -- leaving the tab exactly where it was -- if that path doesn't
/// exist, isn't a folder, or can't be read.
fn try_navigate_to(
    tab_state: &Rc<RefCell<TabState>>,
    requested: PathBuf,
) -> Result<(), error::FileManagerError> {
    let path = normalize(requested);

    if !path.exists() {
        return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
    }

    if !path.is_dir() {
        return Err(error::FileManagerError::NotADirectory);
    }

    // List it up front: a folder that exists but can't be read should be
    // reported as such, not shown as an inexplicably "empty" folder after
    // the tab has already moved into it.
    let _ = fs::read_dir(&path)?;

    let mut s = tab_state.borrow_mut();

    if s.current != path {
        let previous = std::mem::replace(&mut s.current, path);
        s.history.push(previous);
    }

    Ok(())
}

fn navigate_to(tab_state: &Rc<RefCell<TabState>>, requested: PathBuf) {
    let _ = try_navigate_to(tab_state, requested);
}

/// `try_navigate_to`, but tells the user why when it can't be done -- a
/// mistyped path, a file, a folder they aren't allowed into -- instead of
/// silently doing nothing. The main window is looked up through
/// `location_entry`, which is where it's stashed for exactly this sort of
/// helper.
fn navigate_or_report(
    location_entry: &Entry,
    tab_state: &Rc<RefCell<TabState>>,
    requested: PathBuf,
) {
    let shown = requested.display().to_string();

    if let Err(err) = try_navigate_to(tab_state, requested) {
        if let Some(window) = get_obj_data::<_, ApplicationWindow>(location_entry, "main-window") {
            dialogs::show_error(&window, &format!("Can't open \"{shown}\": {err}"));
        }
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
    // `File::uri` percent-encodes the path. A hand-built "file://{path}"
    // breaks on names containing spaces, `#`, `%` or `?`.
    let uri = gio::File::for_path(path).uri();

    if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }

    // Opening something puts it in the recent-files list.
    navigation::recent::record(path);
}

/// Point a tab's view at the right page for `item_count` items: the "empty"
/// label when there are none, otherwise the icon grid or the list view.
fn show_items_page(store: &gio::ListStore, item_count: usize, empty_text: &str) {
    let Some(stack) = get_obj_data::<_, gtk::Stack>(store, "view-stack") else {
        return;
    };

    if let Some(empty_widget) = stack.child_by_name("empty") {
        if let Some(empty_lbl) = empty_widget.downcast_ref::<Label>() {
            empty_lbl.set_label(empty_text);
        }
    }

    let mode = if VIEW_MODE_LIST.load(Ordering::Relaxed) {
        "list"
    } else {
        "files"
    };

    stack.set_visible_child_name(if item_count == 0 { "empty" } else { mode });
}

/// The freedesktop trash's `files` folder: what the sidebar's "Trash" row
/// opens.
fn is_trash_files_dir(path: &Path) -> bool {
    dirs::data_dir().map_or(false, |data| path == data.join("Trash/files").as_path())
}

/// Files were dropped on `destination`. Copy or move them there -- except
/// onto the Trash, where the files are trashed properly (with the metadata
/// the trash format needs, so they can be restored) instead of being tossed
/// into the folder.
fn handle_file_drop(
    job_ui: &JobUi,
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: PathBuf,
) -> bool {
    let Some((operation, sources)) = ui::dnd::plan_drop(target, value, &destination) else {
        return false;
    };

    // Dropping files back into the folder they're already in changes
    // nothing -- and shouldn't pop up a "these already exist" question about
    // the very files being dragged.
    let sources: Vec<PathBuf> = sources
        .into_iter()
        .filter(|source| source.parent() != Some(destination.as_path()))
        .collect();

    if sources.is_empty() {
        return true;
    }

    if is_trash_files_dir(&destination) {
        start_trash_job_ui(
            &job_ui.window,
            &job_ui.notebook,
            &job_ui.ctx,
            &job_ui.location_entry,
            &job_ui.search_entry,
            &job_ui.hidden_toggle,
            &job_ui.sidebar_list,
            &job_ui.watcher_manager,
            sources,
        );

        return true;
    }

    start_paste_job_ui_for(job_ui, operation, sources, destination);

    true
}

/// Copy or cut `paths`: remember which (so a later paste knows whether the
/// originals should go) and put them on the system clipboard for other
/// applications to paste as well.
fn set_clipboard_files(
    widget: &impl IsA<gtk::Widget>,
    ctx: &Rc<RefCell<AppContext>>,
    operation: PendingOp,
    paths: Vec<PathBuf>,
) {
    ui::clipboard::set_files(widget, operation, &paths);
    ctx.borrow_mut().pending = Some((operation, paths));
}

/// Paste the clipboard into the folder the active tab is showing.
///
/// What's pasted comes from the *system* clipboard, so files copied in
/// another application work; the remembered copy-or-cut from
/// `set_clipboard_files` is used only if the clipboard still holds the very
/// files we put there (otherwise someone else has copied since, and what's
/// there is a plain copy).
fn paste_into_current_tab(job_ui: JobUi) {
    let window = job_ui.window.clone();

    ui::clipboard::read_files(&window, move |clipboard_paths| {
        let remembered = job_ui.ctx.borrow_mut().pending.take();

        let plan = match remembered {
            Some((operation, sources))
                if clipboard_paths.is_empty()
                    || ui::clipboard::same_files(&sources, &clipboard_paths) =>
            {
                Some((operation, sources))
            }
            _ if !clipboard_paths.is_empty() => Some((PendingOp::Copy, clipboard_paths)),
            _ => None,
        };

        let Some((operation, sources)) = plan else {
            dialogs::show_info(&job_ui.window, "Nothing to paste", NOTHING_TO_PASTE);
            return;
        };

        let Some((tab_state, _, _, _)) = get_active_widgets(&job_ui.notebook) else {
            return;
        };

        let destination = tab_state.borrow().current.clone();

        // A cut's files have moved once pasted, so there's nothing left on
        // the clipboard worth offering again. (A copy stays: paste it as
        // many times as you like.)
        if matches!(operation, PendingOp::Move) {
            ui::clipboard::clear(&job_ui.window);
        }

        start_paste_job_ui_for(&job_ui, operation, sources, destination);
    });
}

/// Refresh everything a finished file operation may have changed: the tab
/// being viewed, and the split pane if it's open.
fn refresh_after_change(job_ui: &JobUi) {
    if let Some((tab_state, _, store, _)) = get_active_widgets(&job_ui.notebook) {
        refresh_tab(
            &tab_state,
            &store,
            &job_ui.ctx,
            &job_ui.location_entry,
            &job_ui.search_entry,
            &job_ui.hidden_toggle,
            &job_ui.sidebar_list,
        );

        update_watcher(&job_ui.notebook, &job_ui.watcher_manager);
    }

    refresh_split_pane(&job_ui.location_entry);
}

fn refresh_split_pane(location_entry: &Entry) {
    if let Some(split) =
        get_obj_data::<_, Rc<ui::split_pane::SplitPane>>(location_entry, "split-pane")
    {
        if split.container.is_visible() {
            ui::split_pane::refresh_pane(&split.state, &split.store, &split.location_label);
        }
    }
}

/// Copy or move `sources` into `destination` without the progress dialog:
/// `operations::paste_pending` runs on a worker thread (so a big folder
/// doesn't freeze the window) and the result comes back to the GTK thread.
/// Anything that would collide gets a "name (1)" style name instead of
/// overwriting, which is also what makes "Duplicate" work.
fn run_quick_transfer(
    job_ui: JobUi,
    operation: PendingOp,
    sources: Vec<PathBuf>,
    destination: PathBuf,
) {
    if sources.is_empty() {
        return;
    }

    if let Some(status_label) = get_obj_data::<_, Label>(&job_ui.location_entry, "status-label") {
        let verb = if matches!(operation, PendingOp::Move) {
            "Moving"
        } else {
            "Copying"
        };

        status_label.set_label(&format!("{verb} {} item(s)…", sources.len()));
    }

    let (sender, receiver) = async_channel::bounded::<Result<usize, String>>(1);

    std::thread::spawn(move || {
        let result = operations::paste_pending(&destination, operation, &sources)
            .map_err(|err| err.to_string());

        let _ = sender.send_blocking(result);
    });

    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = receiver.recv().await else {
            return;
        };

        match result {
            Ok(0) => dialogs::show_info(
                &job_ui.window,
                "Nothing to do",
                "Everything selected is already in that folder.",
            ),
            Ok(count) => send_job_notification(
                &job_ui.window,
                "Operation complete",
                &format!("{count} item(s) processed"),
            ),
            Err(err) => dialogs::show_error(
                &job_ui.window,
                &format!("Could not finish the operation: {err}"),
            ),
        }

        refresh_after_change(&job_ui);
    });
}

/// Turn a `gtk::FileDialog` result into the reply the D-Bus portal thread is
/// waiting for.
fn portal_reply(result: Result<gio::File, glib::Error>) -> portal::service::PortalResponse {
    use portal::service::PortalResponse;

    match result {
        Ok(file) => match file.path() {
            Some(path) => PortalResponse::Selected(vec![path.display().to_string()]),
            // Something GVfs hasn't given a local path (an sftp:// location
            // with no FUSE mount, say). A caller expecting a filesystem
            // path can't do anything with an empty string, so report the
            // failure instead of pretending a selection was made.
            None => PortalResponse::Error("The selected location has no local path".to_string()),
        },
        // Dismissed, or closed without choosing anything.
        Err(_) => PortalResponse::Cancelled,
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

    // A move removes the originals, so it can't touch protected paths.
    if matches!(operation, PendingOp::Move) {
        if let Err(err) = filesystem::protection::ensure_modifiable(&sources) {
            dialogs::show_error(window, &err.to_string());
            return;
        }
    }

    let tasks = prepare_paste_tasks(window, sources, destination);
    if tasks.is_empty() {
        return;
    }

    let Some(queue) = get_obj_data::<_, JobQueue>(location_entry, "job-queue") else {
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

    // "Move to Trash" on `/usr` or your home folder is just a slower way of
    // deleting them.
    if let Err(err) = filesystem::protection::ensure_modifiable(&paths) {
        dialogs::show_error(window, &err.to_string());
        return;
    }

    let Some(queue) = get_obj_data::<_, JobQueue>(location_entry, "job-queue") else {
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

    let archive_path = operations::archive::default_archive_path(&destination_dir, &sources, "zip");

    let Some(queue) = get_obj_data::<_, JobQueue>(location_entry, "job-queue") else {
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

    let Some(queue) = get_obj_data::<_, JobQueue>(location_entry, "job-queue") else {
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

    let Some(queue) = get_obj_data::<_, JobQueue>(location_entry, "job-queue") else {
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

impl JobUi {
    #[allow(clippy::too_many_arguments)]
    fn new(
        window: &ApplicationWindow,
        notebook: &Notebook,
        ctx: &Rc<RefCell<AppContext>>,
        location_entry: &Entry,
        search_entry: &SearchEntry,
        hidden_toggle: &CheckButton,
        sidebar_list: &ListBox,
        watcher_manager: &Rc<RefCell<filesystem::watcher::WatcherManager>>,
    ) -> Self {
        Self {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        }
    }

    /// Queue `request` to run after any job already in progress.
    fn enqueue(&self, request: JobRequest) {
        let Some(queue) = get_obj_data::<_, JobQueue>(&self.location_entry, "job-queue") else {
            return;
        };

        enqueue_job(&queue, request, self.clone());
    }
}

/// `start_paste_job_ui` for callers that already hold a `JobUi`.
fn start_paste_job_ui_for(
    job_ui: &JobUi,
    operation: PendingOp,
    sources: Vec<PathBuf>,
    destination: PathBuf,
) {
    start_paste_job_ui(
        &job_ui.window,
        &job_ui.notebook,
        &job_ui.ctx,
        &job_ui.location_entry,
        &job_ui.search_entry,
        &job_ui.hidden_toggle,
        &job_ui.sidebar_list,
        &job_ui.watcher_manager,
        operation,
        sources,
        destination,
    );
}

fn start_compress_tar_gz_job_ui(job_ui: &JobUi, sources: Vec<PathBuf>, destination_dir: PathBuf) {
    if sources.is_empty() {
        return;
    }

    let archive_path =
        operations::archive::default_archive_path(&destination_dir, &sources, "tar.gz");

    job_ui.enqueue(JobRequest::CompressTarGz {
        sources,
        archive_path,
    });
}

/// Permanently delete `paths`, after asking. Nothing goes to the Trash:
/// this is for things that can't or shouldn't be trashed, or that the user
/// simply wants gone. Protected paths are refused before the question is
/// even asked, and a system location gets an extra warning.
fn confirm_and_delete_permanently(job_ui: &JobUi, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }

    if let Err(err) = filesystem::protection::ensure_modifiable(&paths) {
        dialogs::show_error(&job_ui.window, &err.to_string());
        return;
    }

    let mut message = if let [only] = paths.as_slice() {
        format!(
            "\"{}\" will be deleted permanently. This can't be undone.",
            only.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| only.display().to_string())
        )
    } else {
        format!(
            "{} items will be deleted permanently. This can't be undone.",
            paths.len()
        )
    };

    if paths
        .iter()
        .any(|path| filesystem::protection::is_system_area(path))
    {
        message.push_str(
            "\n\nThis is in a system location: deleting system files can break your system.",
        );
    }

    let window = job_ui.window.clone();
    let job_ui = job_ui.clone();

    dialogs::confirm_then(
        &window,
        "Delete Permanently?",
        &message,
        "Delete",
        true,
        move || {
            job_ui.enqueue(JobRequest::Delete {
                paths: paths.clone(),
            });
        },
    );
}

/// How to redo a failed job as administrator, and what to ask the user.
#[derive(Clone)]
struct ElevatedRetry {
    operation: operations::privileged::Operation,
    prompt: String,
}

/// Did this job fail for lack of permission -- the one kind of failure
/// that running as administrator can fix?
fn is_permission_error(message: &str) -> bool {
    message.contains("Permission denied") || message.contains("Operation not permitted")
}

fn is_permission_denied(err: &error::FileManagerError) -> bool {
    matches!(
        err,
        error::FileManagerError::Io(io_err)
            if io_err.kind() == std::io::ErrorKind::PermissionDenied
    )
}

/// The administrator equivalent of `request`, if it has one.
fn elevated_retry_for(request: &JobRequest) -> Option<ElevatedRetry> {
    use operations::jobs::ConflictAction;
    use operations::privileged::{Operation, PasteEntry, PasteKind};

    match request {
        JobRequest::Paste { operation, tasks } => {
            let entries: Vec<PasteEntry> = tasks
                .iter()
                .filter(|task| task.action != ConflictAction::Skip)
                .map(|task| PasteEntry {
                    replace: task.action == ConflictAction::Replace,
                    source: task.source.clone(),
                    target: task.destination.clone(),
                })
                .collect();

            if entries.is_empty() {
                return None;
            }

            let kind = match operation {
                PendingOp::Copy => PasteKind::Copy,
                PendingOp::Move => PasteKind::Move,
            };

            Some(ElevatedRetry {
                operation: Operation::Paste { kind, entries },
                prompt:
                    "MITOS Files doesn't have permission to do this. Retry it as administrator?"
                        .to_string(),
            })
        }
        JobRequest::Delete { paths } => Some(ElevatedRetry {
            operation: Operation::Delete(paths.clone()),
            prompt: "Deleting these items needs administrator permission. Delete them as \
                     administrator? This can't be undone."
                .to_string(),
        }),
        // Root has no business quietly filling *its own* trash can with the
        // user's files, so a failed "move to Trash" is offered as what it
        // really is: a permanent deletion.
        JobRequest::Trash { paths } => Some(ElevatedRetry {
            operation: Operation::Delete(paths.clone()),
            prompt: "These items couldn't be moved to the Trash (permission denied). Delete them \
                     permanently as administrator instead? This can't be undone."
                .to_string(),
        }),
        _ => None,
    }
}

/// A rename or create failed. If it was for lack of permission, offer to
/// redo it as administrator (`retry`); any other reason is just reported.
fn report_or_elevate(
    job_ui: &JobUi,
    what: &str,
    err: &error::FileManagerError,
    retry: operations::privileged::Operation,
) {
    if is_permission_denied(err) {
        offer_elevated_retry(
            job_ui,
            &ElevatedRetry {
                operation: retry,
                prompt: format!("You don't have permission to {what}. Retry it as administrator?"),
            },
            &err.to_string(),
        );
    } else {
        dialogs::show_error(&job_ui.window, &format!("Could not {what}: {err}"));
    }
}

/// Explain that an operation needs administrator permission and, if the
/// user agrees, run it that way.
fn offer_elevated_retry(job_ui: &JobUi, retry: &ElevatedRetry, error: &str) {
    let message = format!("{}\n\n({error})", retry.prompt);
    let operation = retry.operation.clone();
    let window = job_ui.window.clone();
    let job_ui = job_ui.clone();

    dialogs::confirm_then(
        &window,
        "Administrator Permission Needed",
        &message,
        "Retry as Administrator",
        true,
        move || run_elevated(job_ui.clone(), operation.clone()),
    );
}

/// Run `operation` as administrator on a worker thread (the password prompt
/// belongs to the elevation command and blocks until answered) and report
/// the outcome.
fn run_elevated(job_ui: JobUi, operation: operations::privileged::Operation) {
    if let Some(status_label) = get_obj_data::<_, Label>(&job_ui.location_entry, "status-label") {
        status_label.set_label("Waiting for administrator authentication\u{2026}");
    }

    let (sender, receiver) = async_channel::bounded::<Result<(), String>>(1);

    std::thread::spawn(move || {
        let _ = sender.send_blocking(operations::privileged::run_elevated(&operation));
    });

    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = receiver.recv().await else {
            return;
        };

        match result {
            Ok(()) => send_job_notification(
                &job_ui.window,
                "Done as administrator",
                "The operation completed.",
            ),
            Err(err) => dialogs::show_error(
                &job_ui.window,
                &format!("Couldn't complete the operation as administrator: {err}"),
            ),
        }

        refresh_after_change(&job_ui);
    });
}

fn prepare_paste_tasks(
    window: &ApplicationWindow,
    sources: Vec<PathBuf>,
    destination: PathBuf,
) -> Vec<operations::jobs::PasteTask> {
    use operations::jobs::{ConflictAction, ConflictPolicy, PasteTask};

    let conflict_count = sources
        .iter()
        .filter(|source| {
            source
                .file_name()
                .map(|name| destination.join(name).exists())
                .unwrap_or(false)
        })
        .count();

    let policy = if conflict_count > 0 {
        dialogs::choose_conflict_policy(window, conflict_count)
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
    } else if let Some(status_label) = get_obj_data::<_, Label>(&ui.location_entry, "status-label")
    {
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

    // If this job fails for lack of permission, this is how to redo it as
    // administrator (`None` for jobs where that makes no sense).
    let elevated_retry = elevated_retry_for(&request);

    let (handle, title) = match request {
        JobRequest::Paste { operation, tasks } => {
            let handle = operations::jobs::start_paste_job(operation, tasks, sender);
            (handle, "File Operation")
        }
        JobRequest::Trash { paths } => {
            let handle = operations::jobs::start_trash_job(paths, sender);
            (handle, "Trash")
        }
        JobRequest::Delete { paths } => {
            let handle = operations::jobs::start_delete_job(paths, sender);
            (handle, "Delete")
        }
        JobRequest::CompressZip {
            sources,
            archive_path,
        } => {
            let handle = operations::archive::start_compress_zip_job(sources, archive_path, sender);
            (handle, "Compress")
        }
        JobRequest::CompressTarGz {
            sources,
            archive_path,
        } => {
            let handle =
                operations::archive::start_compress_tar_gz_job(sources, archive_path, sender);
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

        if let Err(ref err) = result {
            if !err.contains("Cancelled") {
                match &elevated_retry {
                    // Not allowed to: offer to do it as administrator.
                    Some(retry) if is_permission_error(err) => {
                        offer_elevated_retry(&ui_for_done, retry, err);
                    }
                    _ => dialogs::show_error(&window_error, &format!("Job failed: {err}")),
                }
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

        // Copies/moves/trashing may have touched the split pane's folder.
        refresh_split_pane(&location_entry);

        start_next_job(&queue_for_done, ui_for_done.clone());
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
    let (current, search_query, show_hidden) = {
        let mut s = tab_state.borrow_mut();

        // An explicit refresh goes back to showing the folder itself.
        s.showing_results = false;

        (s.current.clone(), s.search_query.clone(), s.show_hidden)
    };

    if let Some(crumbs) = get_obj_data::<_, GtkBox>(location_entry, "path-crumbs") {
        ui::path_bar::update(&crumbs, location_entry, &current);
    } else if location_entry.text().as_str() != current.display().to_string() {
        location_entry.set_text(&current.display().to_string());
    }

    if search_entry.text().as_str() != search_query {
        search_entry.set_text(&search_query);
    }

    if hidden_toggle.is_active() != show_hidden {
        hidden_toggle.set_active(show_hidden);
    }

    // Back / Forward / Up grey out when there's nowhere for them to go.
    {
        let s = tab_state.borrow();

        if let Some(button) = get_obj_data::<_, Button>(location_entry, "back-btn") {
            button.set_sensitive(s.history.can_go_back());
        }

        if let Some(button) = get_obj_data::<_, Button>(location_entry, "forward-btn") {
            button.set_sensitive(s.history.can_go_forward());
        }

        if let Some(button) = get_obj_data::<_, Button>(location_entry, "up-btn") {
            button.set_sensitive(current.parent().is_some());
        }
    }

    // The folder's contents are read on a worker thread; the view is filled
    // in when they arrive (`finish_directory_load`).
    start_directory_load(tab_state, store, ctx, location_entry, sidebar_list);
}

/// What a background directory listing hands back to the GTK thread.
struct LoadedDirectory {
    items: Vec<directory::Item>,
    /// May the current user create things here? (`access(2)` can block on a
    /// slow mount, so it's asked here with the listing, off the GTK thread.)
    writable: bool,
    /// An administrator-only location (`/etc`, `/usr`, ...).
    system_area: bool,
    /// Free space on the folder's filesystem, already formatted.
    free: String,
}

/// The blocking half of showing a folder -- listing it, filtering, sorting,
/// asking about permissions and free space -- with no GTK in it, so it can
/// run on a worker thread.
fn load_directory(path: &Path, show_hidden: bool, query: &str) -> LoadedDirectory {
    let mut items = directory::read_items(path, show_hidden);

    if !query.is_empty() {
        let query = query.to_lowercase();
        items.retain(|item| item.name.to_lowercase().contains(&query));
    }

    LoadedDirectory {
        items,
        writable: filesystem::access::can_write(path),
        system_area: filesystem::protection::is_system_area(path),
        free: metadata::free_space_string(path),
    }
}

/// Start listing the tab's folder in the background.
///
/// Opening a folder used to read and describe every file on the GTK thread,
/// so a big folder (or a slow network mount) froze the whole window. Now the
/// window stays live: this starts a worker, and when it reports back
/// `finish_directory_load` fills the view.
///
/// Refreshes that arrive while a listing is running don't start another one
/// (a burst of file-watcher events would otherwise pile up threads); the
/// running one notices it's out of date when it finishes and starts over
/// with the latest state.
fn start_directory_load(
    tab_state: &Rc<RefCell<TabState>>,
    store: &gio::ListStore,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    sidebar_list: &ListBox,
) {
    let generation = {
        let mut s = tab_state.borrow_mut();
        s.load_generation += 1;

        if s.load_in_flight {
            return;
        }

        s.load_in_flight = true;
        s.load_generation
    };

    let (current, query, show_hidden, folder_changed) = {
        let s = tab_state.borrow();
        (
            s.current.clone(),
            s.search_query.clone(),
            s.show_hidden,
            s.loaded_path != s.current,
        )
    };

    // Going to a different folder: say so at once, rather than leaving the
    // old folder's files under the new folder's name until the listing
    // arrives. Refreshing the folder already on screen keeps its rows where
    // they are until the new listing is ready, so it doesn't flicker.
    if folder_changed {
        store.remove_all();
        show_items_page(store, 0, "Loading\u{2026}");
    }

    let (sender, receiver) = async_channel::bounded::<LoadedDirectory>(1);

    {
        let current = current.clone();
        let query = query.clone();

        std::thread::spawn(move || {
            let _ = sender.send_blocking(load_directory(&current, show_hidden, &query));
        });
    }

    let tab_state = tab_state.clone();
    let store = store.clone();
    let ctx = ctx.clone();
    let location_entry = location_entry.clone();
    let sidebar_list = sidebar_list.clone();

    glib::MainContext::default().spawn_local(async move {
        let loaded = receiver.recv().await;

        let (stale, showing_results) = {
            let mut s = tab_state.borrow_mut();
            s.load_in_flight = false;
            (s.load_generation != generation, s.showing_results)
        };

        let Ok(loaded) = loaded else {
            return;
        };

        // Search results arrived while this listing was loading: leave them.
        if showing_results {
            return;
        }

        if stale {
            start_directory_load(&tab_state, &store, &ctx, &location_entry, &sidebar_list);
            return;
        }

        finish_directory_load(
            &tab_state,
            &store,
            &ctx,
            &location_entry,
            &sidebar_list,
            generation,
            &current,
            &query,
            loaded,
        );
    });
}

/// Show a finished listing: fill the view, grey out what can't be done in a
/// read-only folder, refresh the sidebar if anything in it changed, and
/// update the status bar.
#[allow(clippy::too_many_arguments)]
fn finish_directory_load(
    tab_state: &Rc<RefCell<TabState>>,
    store: &gio::ListStore,
    ctx: &Rc<RefCell<AppContext>>,
    location_entry: &Entry,
    sidebar_list: &ListBox,
    generation: u64,
    current: &Path,
    query: &str,
    loaded: LoadedDirectory,
) {
    let LoadedDirectory {
        items,
        writable,
        system_area,
        free,
    } = loaded;

    let item_count = items.len();

    tab_state.borrow_mut().loaded_path = current.to_path_buf();

    // The first screenful goes in now and the rest follows in chunks; if
    // another refresh starts before the last chunk is in, the remainder is
    // dropped (the newer listing replaces everything anyway).
    let still_current = {
        let tab_state = tab_state.clone();
        move || tab_state.borrow().load_generation == generation
    };

    grid_view::render_progressive(store, items, still_current);

    show_items_page(
        store,
        item_count,
        if query.is_empty() {
            "This folder is empty"
        } else {
            "No results found"
        },
    );

    // Permission awareness: don't offer to create or paste into a folder
    // that can't be written to.
    for key in ["new-folder-btn", "new-file-btn", "paste-btn"] {
        if let Some(button) = get_obj_data::<_, Button>(location_entry, key) {
            button.set_sensitive(writable);
        }
    }

    // The sidebar only needs rebuilding when something it shows changed
    // (bookmarks, drives, recent files) -- not on every folder change, which
    // is what it used to do.
    if let Some(win) = get_obj_data::<_, ApplicationWindow>(location_entry, "main-window") {
        let signature = sidebar::signature(&ctx.borrow().bookmarks);

        if get_obj_data::<_, u64>(sidebar_list, "sidebar-signature") != Some(signature) {
            sidebar::build(sidebar_list, &ctx.borrow().bookmarks, &win);
            set_obj_data(sidebar_list, "sidebar-signature", signature);
        }
    }

    if let Some(status_label) = get_obj_data::<_, Label>(location_entry, "status-label") {
        let mut notes = String::new();

        if !writable {
            notes.push_str(" \u{b7} read-only");
        }

        if system_area {
            notes.push_str(" \u{b7} system location");
        }

        status_label.set_label(&format!(
            "{} \u{b7} {} items \u{b7} {} free{}",
            current.display(),
            item_count,
            free,
            notes
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

    set_obj_data(&store, "view-stack", view_stack.clone());

    set_obj_data(&page_widget, "tab-state", tab_state.clone());
    set_obj_data(&page_widget, "grid-view", grid.clone());
    set_obj_data(&page_widget, "list-store", store.clone());
    set_obj_data(&page_widget, "selection-model", selection.clone());

    // Update selection status
    {
        let selection_label: Option<Label> = get_obj_data(location_entry, "selection-label");

        let preview_box: Option<GtkBox> = get_obj_data(location_entry, "preview-box");

        let store_for_selection = store.clone();

        selection.connect_selection_changed(move |selection, _position, _n_items| {
            let selected = grid_view::selected_items(selection, &store_for_selection);

            if let Some(selection_label) = selection_label.clone() {
                if selected.is_empty() {
                    selection_label.set_label("");
                } else {
                    // A folder's own "size" is just its directory entry, so
                    // only files are added up (Properties measures folders).
                    let total: u64 = selected
                        .iter()
                        .filter(|item| !item.is_dir())
                        .map(|item| item.size())
                        .sum();

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

    // Dropping files onto a tab's header puts them in that tab's folder.
    {
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };
        let tab_state = tab_state.clone();

        let drop_target = ui::dnd::new_file_drop_target();

        drop_target.connect_drop(move |target, value, _x, _y| {
            let destination = tab_state.borrow().current.clone();

            handle_file_drop(&job_ui, target, value, destination)
        });

        tab_label.add_controller(drop_target);
    }

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
                notebook.remove_page(Some(page_num));
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
                            navigate_or_report(&location_entry, &tab_state, item_obj.get_path());
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

            let file_list = gtk::gdk::FileList::from_array(&files);
            let provider = gtk::gdk::ContentProvider::for_value(&file_list.to_value());
            Some(provider)
        });
        grid.add_controller(drag_source);
    }

    // 3. Drop Target: drop files into the folder being shown. Copy or move
    // follows the modifier keys, or the filesystem when there's none (see
    // `ui::dnd::choose_operation`).
    {
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };
        let tab_state = tab_state.clone();

        let drop_target = ui::dnd::new_file_drop_target();

        drop_target.connect_drop(move |target, value, _x, _y| {
            let destination = tab_state.borrow().current.clone();

            handle_file_drop(&job_ui, target, value, destination)
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

        // 1. Clone the grid for the closure (cheap reference count increment)
        let grid_for_closure = grid.clone();

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let items = grid_view::selected_items(&selection, &store);
            if !items.is_empty() {
                show_context_menu(
                    &window,
                    &notebook,
                    &ctx,
                    &grid_for_closure,
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
                        navigate_or_report(&location_entry, &tab_state, item_obj.get_path());
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

        let list_view_for_closure = list_view.clone();
        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let items = grid_view::selected_items(&selection, &store);
            if !items.is_empty() {
                show_context_menu(
                    &window,
                    &notebook,
                    &ctx,
                    &list_view_for_closure,
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

            let file_list = gtk::gdk::FileList::from_array(&files);
            Some(gtk::gdk::ContentProvider::for_value(&file_list.to_value()))
        });

        list_view.add_controller(drag_source);
    }

    // ---- List view: drop target (into current directory) ----
    {
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };
        let tab_state = tab_state.clone();

        let drop_target = ui::dnd::new_file_drop_target();

        drop_target.connect_drop(move |target, value, _x, _y| {
            let destination = tab_state.borrow().current.clone();

            handle_file_drop(&job_ui, target, value, destination)
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

fn build_ui(
    app: &Application,
    initial_args: &[String],
    portal_rx: Option<std::sync::mpsc::Receiver<portal::service::PortalRequest>>,
    desktop_rx: Option<std::sync::mpsc::Receiver<desktop::service::DesktopRequest>>,
) {
    let window = ui::window::create_window(app, "MITOS Files");

    ui::accessibility::setup_widget_accessibility(&window);

    let root = GtkBox::new(Orientation::Vertical, 6);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let ctx = Rc::new(RefCell::new(AppContext::new()));

    // Settings are loaded once for the whole process, before the first
    // window is built (see `main`) -- re-loading here on every window
    // would reset anything the user changed at runtime (e.g. toggling
    // List view) back to whatever's on disk. `VIEW_MODE_LIST` etc. are
    // read fresh below regardless, so a new window still reflects
    // whatever the current live state is.

    // Start config watcher
    let (config_tx, config_rx) = async_channel::unbounded();
    let _config_watcher = config::watcher::ConfigWatcher::start(config_tx);

    let theme_mode = config::settings::theme_mode();
    ui::theme::apply_theme(&WidgetExt::display(&window), theme_mode);

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

    // Narrow a search to one kind of file. Entry 0 is "no filter"; the rest
    // are `FileTypeFilter::all()` in order, labelled by `FileTypeFilter::label`.
    let search_type_dropdown = {
        let mut labels: Vec<&str> = vec!["All types"];
        labels.extend(
            search::filters::FileTypeFilter::all()
                .iter()
                .map(|filter| filter.label()),
        );

        gtk::DropDown::from_strings(&labels)
    };

    // Also look inside text files, not just at their names.
    let search_content_toggle = CheckButton::with_label("Contents");

    toolbar1.append(&back_btn);
    toolbar1.append(&forward_btn);
    toolbar1.append(&up_btn);
    toolbar1.append(&home_btn);
    toolbar1.append(&new_window_btn);
    toolbar1.append(&bookmark_btn);
    toolbar1.append(&location_bar);
    toolbar1.append(&search_entry);
    toolbar1.append(&search_recursive_toggle);
    toolbar1.append(&search_type_dropdown);
    toolbar1.append(&search_content_toggle);
    toolbar1.append(&help_btn);

    let toolbar2 = GtkBox::new(Orientation::Horizontal, 6);
    let new_folder_btn = Button::with_label("New Folder");
    let new_file_btn = Button::with_label("New File");
    let rename_btn = Button::with_label("Rename");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let list_toggle = CheckButton::with_label("List");
    list_toggle.set_active(VIEW_MODE_LIST.load(Ordering::Relaxed));
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
    {
        use ui::accessibility::{make_button_accessible, make_entry_accessible};

        make_button_accessible(&back_btn, "Go back (Alt+Left)");
        make_button_accessible(&forward_btn, "Go forward (Alt+Right)");
        make_button_accessible(&up_btn, "Go to parent directory");
        make_button_accessible(&home_btn, "Go to home directory");
        make_button_accessible(&new_window_btn, "Open a new window");
        make_button_accessible(
            &bookmark_btn,
            "Bookmark the current directory (click again to remove the bookmark)",
        );
        make_button_accessible(&new_folder_btn, "Create new folder");
        make_button_accessible(&new_file_btn, "Create new file");
        make_button_accessible(&rename_btn, "Rename selected item (F2)");
        make_button_accessible(&copy_btn, "Copy selected items (Ctrl+C)");
        make_button_accessible(&move_btn, "Move selected items (Ctrl+X)");
        make_button_accessible(&paste_btn, "Paste items (Ctrl+V)");
        make_button_accessible(&trash_btn, "Move to trash (Delete)");
        make_button_accessible(&open_trash_btn, "View trash contents");
        make_button_accessible(&settings_btn, "Open settings");
        make_button_accessible(&preview_toggle, "Toggle file preview panel");
        make_button_accessible(&split_toggle, "Toggle split view");
        make_button_accessible(&tree_toggle, "Toggle tree view sidebar");
        make_button_accessible(&hidden_toggle, "Show hidden files (Ctrl+H)");
        make_button_accessible(&help_btn, "Keyboard shortcuts help");
        make_button_accessible(&list_toggle, "Show files as a list instead of icons");
        make_button_accessible(&search_recursive_toggle, "Search inside subfolders too");
        make_button_accessible(&search_type_dropdown, "Only look for this kind of file");
        make_button_accessible(
            &search_content_toggle,
            "Also search inside text files (slower)",
        );

        make_entry_accessible(&location_entry, "Location -- type a path and press Enter");
        make_entry_accessible(
            &search_entry,
            "Search -- press Enter to search, empty the box to go back",
        );
    }

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

    // Shared (Rc) so the toolbar toggle, the pane's own right-click menu and
    // the main view's context menu can all reach it.
    let split_pane = Rc::new(ui::split_pane::build(locations::home_dir()));
    split_pane.container.set_visible(false);
    content.append(&split_pane.container);

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

    set_obj_data(&location_entry, "path-crumbs", path_crumbs.clone());
    set_obj_data(&location_entry, "location-stack", location_stack.clone());
    set_obj_data(&location_entry, "status-label", status_label.clone());
    set_obj_data(&location_entry, "selection-label", selection_label.clone());
    set_obj_data(&location_entry, "job-queue", job_queue.clone());
    set_obj_data(&location_entry, "preview-panel", preview_scrolled.clone());
    set_obj_data(&location_entry, "new-folder-btn", new_folder_btn.clone());
    set_obj_data(&location_entry, "new-file-btn", new_file_btn.clone());
    set_obj_data(&location_entry, "paste-btn", paste_btn.clone());
    set_obj_data(&location_entry, "split-pane", split_pane.clone());
    set_obj_data(&location_entry, "split-toggle", split_toggle.clone());
    set_obj_data(&location_entry, "back-btn", back_btn.clone());
    set_obj_data(&location_entry, "forward-btn", forward_btn.clone());
    set_obj_data(&location_entry, "up-btn", up_btn.clone());
    set_obj_data(&location_entry, "preview-box", preview_box.clone());
    set_obj_data(&location_entry, "main-window", window.clone());

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

    // Split pane: right-click menu -- open the selection, or copy / move it
    // into the folder the active tab is showing (the other direction, tab ->
    // pane, is in the main context menu).
    {
        let split_for_menu = split_pane.clone();
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let paths = split_for_menu.selected_paths();

            if !paths.is_empty() {
                show_split_context_menu(&job_ui, &split_for_menu, paths, x, y);
            }
        });

        split_pane.grid.add_controller(right_click);
    }

    // Split pane: files can be dragged out of it and dropped onto it (into
    // whichever folder it is showing) -- the same copy/move rules as
    // everywhere else.
    {
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };
        let split_for_drop = split_pane.clone();

        let drop_target = ui::dnd::new_file_drop_target();

        drop_target.connect_drop(move |target, value, _x, _y| {
            let destination = split_for_drop.state.borrow().current.clone();

            handle_file_drop(&job_ui, target, value, destination)
        });

        split_pane.grid.add_controller(drop_target);

        let drag_source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
            .build();
        let split_for_drag = split_pane.clone();

        drag_source.connect_prepare(move |_source, _x, _y| {
            let paths = split_for_drag.selected_paths();

            if paths.is_empty() {
                return None;
            }

            let files: Vec<gio::File> =
                paths.iter().map(|path| gio::File::for_path(path)).collect();
            let file_list = gtk::gdk::FileList::from_array(&files);

            Some(gtk::gdk::ContentProvider::for_value(&file_list.to_value()))
        });

        split_pane.grid.add_controller(drag_source);
    }

    // Poll for config changes
    {
        let window = window.clone();
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
                ui::theme::apply_theme(&WidgetExt::display(&window), theme_mode);

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

            if let Some(focus) = gtk::prelude::GtkWindowExt::focus(&window) {
                if focus.downcast_ref::<gtk::Entry>().is_some()
                    || focus.downcast_ref::<gtk::SearchEntry>().is_some()
                {
                    return glib::Propagation::Proceed;
                }
            }

            let alt = modifier.contains(gtk::gdk::ModifierType::ALT_MASK);
            let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
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
                k if ctrl && k == gtk::gdk::Key::f => {
                    search_entry.grab_focus();
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::c => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if !selected.is_empty() {
                            let paths: Vec<PathBuf> =
                                selected.iter().map(|item| item.get_path()).collect();
                            set_clipboard_files(&window, &ctx, PendingOp::Copy, paths);
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
                            set_clipboard_files(&window, &ctx, PendingOp::Move, paths);
                        }
                    }
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::v => {
                    paste_into_current_tab(JobUi {
                        window: window.clone(),
                        notebook: notebook.clone(),
                        ctx: ctx.clone(),
                        location_entry: location_entry.clone(),
                        search_entry: search_entry.clone(),
                        hidden_toggle: hidden_toggle.clone(),
                        sidebar_list: sidebar_list.clone(),
                        watcher_manager: watcher_manager.clone(),
                    });
                    return glib::Propagation::Stop;
                }
                k if ctrl && k == gtk::gdk::Key::d => {
                    // Duplicate: copy each selected item next to the original
                    // under a free "name (1)" name.
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        let paths: Vec<PathBuf> =
                            selected.iter().map(|item| item.get_path()).collect();
                        let destination = paths
                            .first()
                            .and_then(|path| path.parent())
                            .map(|parent| parent.to_path_buf());

                        if let Some(destination) = destination {
                            run_quick_transfer(
                                JobUi {
                                    window: window.clone(),
                                    notebook: notebook.clone(),
                                    ctx: ctx.clone(),
                                    location_entry: location_entry.clone(),
                                    search_entry: search_entry.clone(),
                                    hidden_toggle: hidden_toggle.clone(),
                                    sidebar_list: sidebar_list.clone(),
                                    watcher_manager: watcher_manager.clone(),
                                },
                                PendingOp::Copy,
                                paths,
                                destination,
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
                        notebook.remove_page(Some(page_num));
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
                // Up one folder: Alt+Up, or Backspace as in most file managers.
                k if (alt && k == gtk::gdk::Key::Up) || k == gtk::gdk::Key::BackSpace => {
                    if let Some((tab_state, _, store, _)) = &active {
                        let parent = tab_state
                            .borrow()
                            .current
                            .parent()
                            .map(|parent| parent.to_path_buf());

                        if let Some(parent) = parent {
                            navigate_to(tab_state, parent);
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
                _ if key == gtk::gdk::Key::Delete => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if !selected.is_empty() {
                            let paths: Vec<PathBuf> =
                                selected.iter().map(|item| item.get_path()).collect();

                            if shift {
                                // Shift+Delete skips the Trash -- after asking.
                                confirm_and_delete_permanently(
                                    &JobUi::new(
                                        &window,
                                        &notebook,
                                        &ctx,
                                        &location_entry,
                                        &search_entry,
                                        &hidden_toggle,
                                        &sidebar_list,
                                        &watcher_manager,
                                    ),
                                    paths,
                                );
                            } else {
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
                    }
                    return glib::Propagation::Stop;
                }
                _ if key == gtk::gdk::Key::F2 => {
                    if let Some((_, _, store, selection)) = &active {
                        let selected = grid_view::selected_items(selection, store);
                        if selected.len() == 1 {
                            let item = selected[0].clone();
                            let source = item.get_path();
                            let initial_name = item.name();
                            let window_dialog = window.clone();
                            let window_err = window.clone();
                            let notebook_clone = notebook.clone();
                            let ctx_clone = ctx.clone();
                            let location_entry_clone = location_entry.clone();
                            let search_entry_clone = search_entry.clone();
                            let hidden_toggle_clone = hidden_toggle.clone();
                            let sidebar_list_clone = sidebar_list.clone();
                            let watcher_manager_clone = watcher_manager.clone();

                            dialogs::show_text_dialog(
                                &window_dialog,
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
                                        report_or_elevate(
                                            &JobUi::new(
                                                &window_err,
                                                &notebook_clone,
                                                &ctx_clone,
                                                &location_entry_clone,
                                                &search_entry_clone,
                                                &hidden_toggle_clone,
                                                &sidebar_list_clone,
                                                &watcher_manager_clone,
                                            ),
                                            "rename",
                                            &err,
                                            operations::privileged::Operation::Rename {
                                                from: source.clone(),
                                                to: source.with_file_name(&name),
                                            },
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
                _ if key == gtk::gdk::Key::F5 => {
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

    // --- Search: Enter runs it; the type dropdown and "Contents" box re-run it ---
    {
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry_clone = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let search_recursive_toggle = search_recursive_toggle.clone();
        let search_type_dropdown = search_type_dropdown.clone();
        let search_content_toggle = search_content_toggle.clone();

        // Cancel flag of the search running right now, if any: starting
        // another one (or emptying the box) stops the old one instead of
        // letting two searches race to fill the grid.
        let active_search: Rc<RefCell<Option<std::sync::Arc<AtomicBool>>>> =
            Rc::new(RefCell::new(None));

        search_entry.connect_activate(move |entry| {
            let query = entry.text().to_string();

            if let Some(previous) = active_search.borrow_mut().take() {
                previous.store(true, Ordering::Relaxed);
            }

            // Entry 0 of the dropdown is "All types"; entry N is
            // `FileTypeFilter::all()[N - 1]`.
            let file_types: Vec<search::filters::FileTypeFilter> = (search_type_dropdown.selected()
                as usize)
                .checked_sub(1)
                .and_then(|index| {
                    search::filters::FileTypeFilter::all()
                        .into_iter()
                        .nth(index)
                })
                .into_iter()
                .collect();

            // Nothing typed and no type chosen: back to the plain folder view.
            if query.is_empty() && file_types.is_empty() {
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
            let match_content = search_content_toggle.is_active();

            let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) else {
                return;
            };

            // A plain name filter on the current folder is what `refresh_tab`
            // already does (and it keeps working as the folder changes).
            // Anything more -- subfolders, file contents, a file type --
            // goes through the search engine.
            if !recursive && !match_content && file_types.is_empty() {
                tab_state.borrow_mut().search_query = query;
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
                return;
            }

            let (root, show_hidden) = {
                let s = tab_state.borrow();
                (s.current.clone(), s.show_hidden)
            };

            let filters = search::filters::SearchFilters {
                query: query.clone(),
                recursive,
                match_file_name: true,
                match_content,
                include_hidden: show_hidden,
                file_types,
                min_size_bytes: None,
                max_size_bytes: None,
            };

            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            *active_search.borrow_mut() = Some(cancel.clone());

            let (tx, rx) = std::sync::mpsc::channel();
            search::engine::start_search(root.clone(), filters, cancel, tx);

            if let Some(status_label) = get_obj_data::<_, Label>(&location_entry, "status-label") {
                status_label.set_label("Searching…");
            }

            let location_entry = location_entry.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                match rx.try_recv() {
                    Ok(results) => {
                        // The tab may have moved to another folder while the
                        // search ran; results for the old one would replace
                        // what it's showing now, so they're dropped.
                        if tab_state.borrow().current != root {
                            return glib::ControlFlow::Break;
                        }

                        let items: Vec<directory::Item> =
                            results.into_iter().map(|result| result.item).collect();
                        let count = items.len();

                        grid_view::render(&store, &items);

                        {
                            let mut s = tab_state.borrow_mut();
                            s.search_query = query.clone();
                            // Results, not a listing: stop any chunked fill of
                            // the folder that was still going, and ignore a
                            // listing still in flight, so neither mixes into
                            // (or replaces) what was just found.
                            s.showing_results = true;
                            s.load_generation += 1;
                        }

                        show_items_page(&store, count, "No results found");

                        if let Some(status_label) =
                            get_obj_data::<_, Label>(&location_entry, "status-label")
                        {
                            let more = if count >= search::engine::MAX_RESULTS {
                                "+"
                            } else {
                                ""
                            };

                            status_label.set_label(&format!(
                                "{count}{more} result(s) for \"{query}\" in {}",
                                root.display()
                            ));
                        }

                        glib::ControlFlow::Break
                    }
                    // The search thread ended without a result: a newer
                    // search cancelled it, and that one owns the grid now.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                }
            });
        });
    }

    // Changing the file-type filter, or ticking "Contents", re-runs whatever
    // is in the search box (an empty box plus a type shows every file of
    // that type).
    {
        let search_entry = search_entry.clone();

        search_type_dropdown.connect_notify_local(Some("selected"), move |_, _| {
            search_entry.emit_by_name::<()>("activate", &[]);
        });
    }

    {
        let search_entry = search_entry.clone();

        search_content_toggle.connect_toggled(move |_| {
            if !search_entry.text().is_empty() {
                search_entry.emit_by_name::<()>("activate", &[]);
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
            if let Some(stack) = get_obj_data::<_, gtk::Stack>(entry, "location-stack") {
                stack.set_visible_child_name("crumbs");
            }

            let text = entry.text().to_string();
            let path = PathBuf::from(text);

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                // A file path typed into the location bar opens the file
                // (as most file managers do) rather than complaining that
                // it isn't a folder.
                if path.is_file() {
                    open_file_default(&path);
                    return;
                }

                navigate_or_report(&location_entry_clone, &tab_state, path);
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

        let right_click = gtk::GestureClick::new();
        let sidebar_list_for_closure = sidebar_list.clone();
        sidebar_list.clone().connect_row_activated(move |_, row| {
            let _ = &sidebar_list_for_closure;

            if row.widget_name() == "action:connect-to-server" {
                if let Some(window) =
                    get_obj_data::<_, ApplicationWindow>(&location_entry, "main-window")
                {
                    let notebook = notebook.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list_for_connect = sidebar_list_for_closure.clone();
                    let watcher_manager = watcher_manager.clone();

                    dialogs::show_connect_to_server(&window, move |path| {
                        if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                            navigate_to(&tab_state, path);
                            refresh_tab(
                                &tab_state,
                                &store,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list_for_connect,
                            );
                            update_watcher(&notebook, &watcher_manager);
                        }
                    });
                }
                return;
            }

            // An unmounted drive: mount it, then open it.
            let row_name = row.widget_name();

            if let Some(volume_key) = row_name.strip_prefix("volume:") {
                let volume = gio::VolumeMonitor::get()
                    .volumes()
                    .into_iter()
                    .find(|volume| sidebar::volume_id(volume) == volume_key);

                if let (Some(volume), Some(window)) = (
                    volume,
                    get_obj_data::<_, ApplicationWindow>(&location_entry, "main-window"),
                ) {
                    let notebook = notebook.clone();
                    let ctx = ctx.clone();
                    let location_entry = location_entry.clone();
                    let search_entry = search_entry.clone();
                    let hidden_toggle = hidden_toggle.clone();
                    let sidebar_list = sidebar_list_for_closure.clone();
                    let watcher_manager = watcher_manager.clone();

                    sidebar::mount_volume(&volume, &window, move |path| {
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
                    });
                }

                return;
            }

            if let Some(path) = sidebar::resolve_click(row) {
                if path.is_file() {
                    open_file_default(&path);
                    return;
                }

                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    navigate_or_report(&location_entry, &tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list_for_closure,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
        sidebar_list.add_controller(right_click);
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

        let sidebar_list_for_closure = sidebar_list.clone();
        right_click.connect_pressed(move |_gesture, _n_press, x, y| {
            let _ = &sidebar_list_for_closure;
            if let Some(row) = sidebar_list_for_closure.row_at_y(y as i32) {
                let name = row.widget_name();
                if let Some(path_str) = name.strip_prefix("bm:") {
                    let path = PathBuf::from(path_str);
                    show_sidebar_context_menu(
                        &window,
                        &notebook,
                        &ctx,
                        &sidebar_list_for_closure,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        path,
                        x,
                        y,
                    );
                } else if name.starts_with("recent:") {
                    show_recent_context_menu(
                        &sidebar_list_for_closure,
                        &location_entry,
                        &ctx,
                        x,
                        y,
                    );
                }
            }
        });
        sidebar_list.add_controller(right_click);
    }

    // Drop files onto a place in the sidebar -- a folder, a bookmark, a
    // drive -- to copy or move them there. (One controller on the whole
    // list, looking up the row under the pointer, so it survives the
    // sidebar being rebuilt.)
    {
        let job_ui = JobUi {
            window: window.clone(),
            notebook: notebook.clone(),
            ctx: ctx.clone(),
            location_entry: location_entry.clone(),
            search_entry: search_entry.clone(),
            hidden_toggle: hidden_toggle.clone(),
            sidebar_list: sidebar_list.clone(),
            watcher_manager: watcher_manager.clone(),
        };
        let sidebar_for_drop = sidebar_list.clone();

        let drop_target = ui::dnd::new_file_drop_target();

        drop_target.connect_drop(move |target, value, _x, y| {
            let Some(row) = sidebar_for_drop.row_at_y(y as i32) else {
                return false;
            };

            let Some(destination) = sidebar::resolve_click(&row) else {
                return false;
            };

            // Rows for recent *files* resolve to a file, not somewhere to
            // put things.
            if !destination.is_dir() {
                return false;
            }

            handle_file_drop(&job_ui, target, value, destination)
        });

        sidebar_list.add_controller(drop_target);
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

        let sidebar_list_for_closure = sidebar_list.clone();
        tree_list.clone().connect_row_activated(move |_, row| {
            let _ = &sidebar_list_for_closure;
            let path_str = row.widget_name();
            if !path_str.is_empty() {
                let path = PathBuf::from(path_str.as_str());

                ui::tree_view::toggle_folder(&tree_state, &tree_list, path.clone());

                if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                    navigate_to(&tab_state, path);
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list_for_closure,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3); // Standard secondary/right-click binding
        sidebar_list.add_controller(right_click);
    }

    // New Window Button
    {
        let app = app.clone();
        new_window_btn.connect_clicked(move |_| {
            // No portal/desktop receivers -- those D-Bus services are
            // started once per process (see `main`) and already owned by
            // the first window; a second `Some` here would mean two
            // windows both polling the same already-drained channel.
            build_ui(&app, &[], None, None);
        });
    }

    // Help Button
    {
        let window = window.clone();
        help_btn.connect_clicked(move |_| {
            let shortcuts = ui::accessibility::setup_keyboard_help();

            let dialog = dialogs::build_dialog(&window, "Keyboard Shortcuts");

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

            dialog.content.append(&grid);

            let close_btn = dialogs::dialog_button(&dialog.button_row, "Close");
            let window_for_close = dialog.window.clone();
            close_btn.connect_clicked(move |_| window_for_close.close());

            dialog.window.present();
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

        let sidebar_list_for_closure = sidebar_list.clone();
        hidden_toggle.clone().connect_toggled(move |toggle| {
            let is_active = toggle.is_active();
            config::settings::set_show_hidden(is_active);

            if let Some((tab_state, _, store, _)) = get_active_widgets(&notebook) {
                let _ = &sidebar_list_for_closure;
                if tab_state.borrow().show_hidden != is_active {
                    tab_state.borrow_mut().show_hidden = is_active;
                    refresh_tab(
                        &tab_state,
                        &store,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list_for_closure,
                    );
                    update_watcher(&notebook, &watcher_manager);
                }
            }
        });
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3); // Standard secondary/right-click binding
        sidebar_list.add_controller(right_click);
    }

    // Right-click Back or Forward: pick any earlier (or later) folder from a
    // list, instead of stepping one at a time.
    for (button, forward) in [(back_btn.clone(), false), (forward_btn.clone(), true)] {
        let job_ui = JobUi::new(
            &window,
            &notebook,
            &ctx,
            &location_entry,
            &search_entry,
            &hidden_toggle,
            &sidebar_list,
            &watcher_manager,
        );

        // Weak: the button owns this gesture, which owns this closure.
        let anchor = button.downgrade();

        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);

        right_click.connect_pressed(move |_gesture, _n_press, _x, _y| {
            if let Some(anchor) = anchor.upgrade() {
                show_history_menu(&anchor, forward, job_ui.clone());
            }
        });

        button.add_controller(right_click);
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
                            report_or_elevate(
                                &JobUi::new(
                                    &window_error,
                                    &notebook,
                                    &ctx,
                                    &location_entry,
                                    &search_entry,
                                    &hidden_toggle,
                                    &sidebar_list,
                                    &watcher_manager,
                                ),
                                "create folder",
                                &err,
                                operations::privileged::Operation::CreateFolder(parent.join(&name)),
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
                            report_or_elevate(
                                &JobUi::new(
                                    &window_error,
                                    &notebook,
                                    &ctx,
                                    &location_entry,
                                    &search_entry,
                                    &hidden_toggle,
                                    &sidebar_list,
                                    &watcher_manager,
                                ),
                                "create file",
                                &err,
                                operations::privileged::Operation::CreateFile(parent.join(&name)),
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
                            report_or_elevate(
                                &JobUi::new(
                                    &window_error,
                                    &notebook,
                                    &ctx,
                                    &location_entry,
                                    &search_entry,
                                    &hidden_toggle,
                                    &sidebar_list,
                                    &watcher_manager,
                                ),
                                "rename",
                                &err,
                                operations::privileged::Operation::Rename {
                                    from: source.clone(),
                                    to: source.with_file_name(&name),
                                },
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
        let notebook = notebook.clone();
        let ctx = ctx.clone();

        copy_btn.connect_clicked(move |_| {
            if let Some((_, _, store, selection)) = get_active_widgets(&notebook) {
                let selected = grid_view::selected_items(&selection, &store);
                if selected.is_empty() {
                    return;
                }
                let paths: Vec<PathBuf> = selected.iter().map(|item| item.get_path()).collect();
                set_clipboard_files(&notebook, &ctx, PendingOp::Copy, paths);
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
                set_clipboard_files(&notebook, &ctx, PendingOp::Move, paths);
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
            paste_into_current_tab(JobUi {
                window: window_error.clone(),
                notebook: notebook.clone(),
                ctx: ctx.clone(),
                location_entry: location_entry.clone(),
                search_entry: search_entry.clone(),
                hidden_toggle: hidden_toggle.clone(),
                sidebar_list: sidebar_list.clone(),
                watcher_manager: watcher_manager.clone(),
            });
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
            if let Some(panel) =
                get_obj_data::<_, gtk::ScrolledWindow>(&location_entry, "preview-panel")
            {
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
        let split_pane = split_pane.clone();

        split_toggle.connect_toggled(move |toggle| {
            split_pane.container.set_visible(toggle.is_active());

            if toggle.is_active() {
                split_pane.focus_grid();
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
                    // Already bookmarked: clicking again takes it back off.
                    bookmarks::remove(&mut c.bookmarks, &current);
                } else {
                    bookmarks::add(&mut c.bookmarks, name, current);
                }
                drop(c);
                if let Some(win) =
                    get_obj_data::<_, gtk::ApplicationWindow>(&location_entry, "main-window")
                {
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
            if let Some(win) =
                get_obj_data::<_, gtk::ApplicationWindow>(&location_entry, "main-window")
            {
                sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
            }

            if let Some(lost_path) = unmounted_path {
                // Every tab -- not just the visible one -- that was browsing
                // the vanished volume goes back home, and forgets its Back /
                // Forward trail, which may be full of places on that volume.
                for page in 0..notebook.n_pages() {
                    let Some(widget) = notebook.nth_page(Some(page)) else {
                        continue;
                    };

                    let Some(state) =
                        get_obj_data::<_, Rc<RefCell<TabState>>>(&widget, "tab-state")
                    else {
                        continue;
                    };

                    let mut s = state.borrow_mut();

                    if s.current.starts_with(&lost_path) {
                        s.current = locations::home_dir();
                        s.history.clear();
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

    // Portal Receiver -- only set up for the window that owns it (see
    // `main`); a window opened via "New Window" gets `None` here, since
    // the D-Bus service itself is only started once per process.
    if let Some(portal_rx) = portal_rx {
        let window = window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || loop {
            match portal_rx.try_recv() {
                Ok(request) => match request {
                    portal::service::PortalRequest::OpenFile { title, response_tx } => {
                        let dialog = gtk::FileDialog::builder().title(&title).modal(true).build();
                        dialog.open(
                            Some(&window),
                            None::<&gtk::gio::Cancellable>,
                            move |result| {
                                let _ = response_tx.send(portal_reply(result));
                            },
                        );
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
                        dialog.save(
                            Some(&window),
                            None::<&gtk::gio::Cancellable>,
                            move |result| {
                                let _ = response_tx.send(portal_reply(result));
                            },
                        );
                    }
                    portal::service::PortalRequest::OpenFolder { title, response_tx } => {
                        let dialog = gtk::FileDialog::builder().title(&title).modal(true).build();
                        dialog.select_folder(
                            Some(&window),
                            None::<&gtk::gio::Cancellable>,
                            move |result| {
                                let _ = response_tx.send(portal_reply(result));
                            },
                        );
                    }
                    portal::service::PortalRequest::SaveFiles {
                        title,
                        current_folder,
                        files,
                        response_tx,
                    } => {
                        // Same folder-picker as OpenFolder above; the part
                        // that's actually specific to SaveFiles is what
                        // happens once a folder comes back -- joining it
                        // with each requested filename so the generic
                        // `uris`-building code in xdg_portal.rs's
                        // `begin_request` (unchanged) has one path per
                        // input file to work with, not just the folder.
                        let mut dialog_builder =
                            gtk::FileDialog::builder().title(&title).modal(true);
                        if let Some(folder) = &current_folder {
                            dialog_builder =
                                dialog_builder.initial_folder(&gtk::gio::File::for_path(folder));
                        }
                        let dialog = dialog_builder.build();
                        dialog.select_folder(
                            Some(&window),
                            None::<&gtk::gio::Cancellable>,
                            move |result| {
                                let reply = match portal_reply(result) {
                                    portal::service::PortalResponse::Selected(folders) => {
                                        match folders.into_iter().next() {
                                            Some(folder) => {
                                                portal::service::PortalResponse::Selected(
                                                    files
                                                        .iter()
                                                        .map(|name| {
                                                            PathBuf::from(&folder)
                                                                .join(name)
                                                                .display()
                                                                .to_string()
                                                        })
                                                        .collect(),
                                                )
                                            }
                                            None => portal::service::PortalResponse::Cancelled,
                                        }
                                    }
                                    other => other,
                                };

                                let _ = response_tx.send(reply);
                            },
                        );
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return glib::ControlFlow::Break
                }
            }
        });
    }

    // Desktop Receiver -- same one-owner-per-process reasoning as the
    // portal receiver above.
    if let Some(desktop_rx) = desktop_rx {
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || loop {
            match desktop_rx.try_recv() {
                Ok(request) => match request {
                    desktop::service::DesktopRequest::GetWallpaper { response_tx } => {
                        let wallpaper = config::shared::SharedConfig::load().wallpaper;
                        let _ = response_tx.send(wallpaper);
                    }
                    desktop::service::DesktopRequest::SetWallpaper { path, response_tx } => {
                        let mut shared_config = config::shared::SharedConfig::load();
                        shared_config.wallpaper = path;
                        let result = shared_config.save().map_err(|err| err.to_string());
                        let _ = response_tx.send(result);
                    }
                    desktop::service::DesktopRequest::OpenDesktopSettings { response_tx } => {
                        let result = Command::new("mitos-settings")
                            .spawn()
                            .map(|_| ())
                            .map_err(|err| err.to_string());
                        let _ = response_tx.send(result);
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return glib::ControlFlow::Break
                }
            }
        });
    }

    window.present();
}

fn show_context_menu<W: IsA<gtk::Widget>>(
    window: &ApplicationWindow,
    notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    parent: &W,
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
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let open_btn = Button::with_label("Open");
    let open_tab_btn = Button::with_label("Open in New Tab");
    let open_with_btn = Button::with_label("Open With");
    let compress_btn = Button::with_label("Compress to ZIP");
    let compress_targz_btn = Button::with_label("Compress to TAR.GZ");
    let extract_btn = Button::with_label("Extract Here");
    let copy_btn = Button::with_label("Copy");
    let move_btn = Button::with_label("Move");
    let duplicate_btn = Button::with_label("Duplicate");
    let copy_to_split_btn = Button::with_label("Copy to Split Pane");
    let move_to_split_btn = Button::with_label("Move to Split Pane");
    let open_in_split_btn = Button::with_label("Open in Split Pane");
    let rename_btn = Button::with_label("Rename");
    let batch_rename_btn = Button::with_label("Batch Rename");
    let link_btn = Button::with_label("Create Link");
    let copy_path_btn = Button::with_label("Copy Path");
    let trash_btn = Button::with_label("Trash");
    let delete_btn = Button::with_label("Delete Permanently\u{2026}");
    delete_btn.add_css_class("destructive-action");
    let properties_btn = Button::with_label("Properties");

    // A link or a duplicate goes beside the original, so it needs the
    // original's folder to be writable. (Rename, Trash and Delete stay
    // enabled: when they fail for lack of permission they offer to retry as
    // administrator.)
    let parent_writable = items
        .first()
        .and_then(|item| {
            item.get_path()
                .parent()
                .map(|parent| filesystem::access::can_write(parent))
        })
        .unwrap_or(true);

    link_btn.set_sensitive(parent_writable);
    duplicate_btn.set_sensitive(parent_writable);

    // The transfer-to-split entries only make sense while the pane is
    // actually showing, so they only appear then; "Open in Split Pane" is
    // for exactly one folder.
    let split_visible =
        get_obj_data::<_, Rc<ui::split_pane::SplitPane>>(location_entry, "split-pane")
            .map_or(false, |split| split.container.is_visible());
    let single_folder = count == 1 && single_item.as_ref().map_or(false, |i| i.is_dir());

    open_btn.set_sensitive(count == 1);
    open_tab_btn.set_sensitive(single_folder);
    open_with_btn.set_sensitive(count == 1 && single_item.as_ref().map_or(false, |i| !i.is_dir()));
    compress_btn.set_sensitive(!items.is_empty());
    extract_btn.set_sensitive(
        count == 1
            && single_item.as_ref().map_or(false, |item| {
                operations::archive::is_supported_archive(&item.get_path())
            }),
    );
    rename_btn.set_sensitive(count == 1);
    batch_rename_btn.set_sensitive(count >= 2);

    menu_box.append(&open_btn);
    menu_box.append(&open_tab_btn);
    menu_box.append(&open_with_btn);
    menu_box.append(&compress_btn);
    menu_box.append(&compress_targz_btn);
    menu_box.append(&extract_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&move_btn);
    menu_box.append(&duplicate_btn);

    if split_visible {
        menu_box.append(&copy_to_split_btn);
        menu_box.append(&move_to_split_btn);
    }

    if single_folder {
        menu_box.append(&open_in_split_btn);
    }

    menu_box.append(&rename_btn);
    menu_box.append(&batch_rename_btn);
    menu_box.append(&link_btn);
    menu_box.append(&copy_path_btn);
    menu_box.append(&trash_btn);
    menu_box.append(&delete_btn);
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

    // With this many entries the menu can be taller than a small window, so
    // it scrolls.
    let scroller = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .max_content_height(480)
        .propagate_natural_height(true)
        .build();
    scroller.set_child(Some(&menu_box));
    popover.set_child(Some(&scroller));

    {
        let popover = popover.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let _watcher_manager = watcher_manager.clone();
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
        let window = window.clone();
        let notebook = notebook.clone();
        let ctx = ctx.clone();
        let location_entry = location_entry.clone();
        let search_entry = search_entry.clone();
        let hidden_toggle = hidden_toggle.clone();
        let sidebar_list = sidebar_list.clone();
        let watcher_manager = watcher_manager.clone();
        let single_item_tab = single_item.clone();

        open_tab_btn.connect_clicked(move |_| {
            popover.popdown();
            let Some(item) = single_item_tab.clone() else {
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
        let open_with_btn_for_closure = open_with_btn.clone();

        open_with_btn.connect_clicked(move |_| {
            popover.popdown();
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
            sub_popover.set_parent(&open_with_btn_for_closure);

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

            let sub_popover_for_closure = sub_popover.clone();
            default_btn.connect_clicked(move |_| {
                sub_popover_for_closure.popdown();
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
        let window = window.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        copy_btn.connect_clicked(move |_| {
            popover.popdown();
            set_clipboard_files(&window, &ctx, PendingOp::Copy, paths.clone());
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let ctx = ctx.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        move_btn.connect_clicked(move |_| {
            popover.popdown();
            set_clipboard_files(&window, &ctx, PendingOp::Move, paths.clone());
        });
    }

    // What the quick transfers below hand to `run_quick_transfer`.
    let job_ui = JobUi {
        window: window.clone(),
        notebook: notebook.clone(),
        ctx: ctx.clone(),
        location_entry: location_entry.clone(),
        search_entry: search_entry.clone(),
        hidden_toggle: hidden_toggle.clone(),
        sidebar_list: sidebar_list.clone(),
        watcher_manager: watcher_manager.clone(),
    };

    // Copy / Move to Split Pane: into whichever folder the split pane is
    // showing. Unlike the plain copy above this used to run on the GTK
    // thread with every error swallowed and any existing file overwritten;
    // now it's off-thread, reports failures, and never overwrites.
    for (button, operation) in [
        (copy_to_split_btn.clone(), PendingOp::Copy),
        (move_to_split_btn.clone(), PendingOp::Move),
    ] {
        let popover = popover.clone();
        let job_ui = job_ui.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        button.connect_clicked(move |_| {
            popover.popdown();

            if let Some(split) = get_obj_data::<_, Rc<ui::split_pane::SplitPane>>(
                &job_ui.location_entry,
                "split-pane",
            ) {
                let destination = split.state.borrow().current.clone();
                run_quick_transfer(job_ui.clone(), operation, paths.clone(), destination);
            }
        });
    }

    // Compress to TAR.GZ: like ZIP, but keeps permissions, timestamps and
    // symlinks, which ZIP does poorly.
    {
        let popover = popover.clone();
        let job_ui = job_ui.clone();
        let sources: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        compress_targz_btn.connect_clicked(move |_| {
            popover.popdown();

            if let Some((tab_state, _, _, _)) = get_active_widgets(&job_ui.notebook) {
                let destination_dir = tab_state.borrow().current.clone();
                start_compress_tar_gz_job_ui(&job_ui, sources.clone(), destination_dir);
            }
        });
    }

    // Create Link: a symbolic link to each selected item, beside it.
    {
        let popover = popover.clone();
        let job_ui = job_ui.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        link_btn.connect_clicked(move |_| {
            popover.popdown();

            let mut first_error: Option<String> = None;

            for path in &paths {
                let Some(directory) = path.parent() else {
                    continue;
                };

                if let Err(err) = operations::link::create_symlink(path, directory) {
                    first_error.get_or_insert_with(|| {
                        format!("Couldn't create a link to \"{}\": {err}", path.display())
                    });
                }
            }

            if let Some(message) = first_error {
                dialogs::show_error(&job_ui.window, &message);
            }

            refresh_after_change(&job_ui);
        });
    }

    // Copy Path: the full path(s) as text, ready to paste into a terminal.
    {
        let popover = popover.clone();
        let window = window.clone();
        let text = items
            .iter()
            .map(|item| item.get_path().display().to_string())
            .collect::<Vec<_>>()
            .join("\n");

        copy_path_btn.connect_clicked(move |_| {
            popover.popdown();
            ui::clipboard::set_text(&window, &text);
        });
    }

    // Delete Permanently: skips the Trash, after confirmation.
    {
        let popover = popover.clone();
        let job_ui = job_ui.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        delete_btn.connect_clicked(move |_| {
            popover.popdown();
            confirm_and_delete_permanently(&job_ui, paths.clone());
        });
    }

    // Open in Split Pane: point the pane at the selected folder and show it.
    {
        let popover = popover.clone();
        let location_entry = location_entry.clone();
        let folder = single_item
            .as_ref()
            .filter(|item| item.is_dir())
            .map(|item| item.get_path());

        open_in_split_btn.connect_clicked(move |_| {
            popover.popdown();

            let (Some(folder), Some(split)) = (
                folder.clone(),
                get_obj_data::<_, Rc<ui::split_pane::SplitPane>>(&location_entry, "split-pane"),
            ) else {
                return;
            };

            split.navigate(folder);

            // Switching the toolbar toggle on shows the pane (its own
            // handler does that and keeps the checkbox honest); if the
            // toggle can't be found, show the pane directly.
            match get_obj_data::<_, CheckButton>(&location_entry, "split-toggle") {
                Some(toggle) => toggle.set_active(true),
                None => split.container.set_visible(true),
            }
        });
    }

    // Duplicate: a copy of each item right beside it, under a free name.
    {
        let popover = popover.clone();
        let job_ui = job_ui.clone();
        let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();

        duplicate_btn.connect_clicked(move |_| {
            popover.popdown();

            let destination = paths
                .first()
                .and_then(|path| path.parent())
                .map(|parent| parent.to_path_buf());

            if let Some(destination) = destination {
                run_quick_transfer(job_ui.clone(), PendingOp::Copy, paths.clone(), destination);
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

            let window_for_dialog = window.clone();
            let window_error = window.clone();
            let notebook = notebook.clone();
            let ctx = ctx.clone();
            let location_entry = location_entry.clone();
            let search_entry = search_entry.clone();
            let hidden_toggle = hidden_toggle.clone();
            let sidebar_list = sidebar_list.clone();
            let watcher_manager = watcher_manager.clone();

            dialogs::show_text_dialog(
                &window_for_dialog,
                "Rename",
                &initial_name,
                "Rename",
                move |name| {
                    if name.is_empty() {
                        return;
                    }
                    if let Err(err) = operations::rename::rename_path(&source, &name) {
                        report_or_elevate(
                            &JobUi::new(
                                &window_error,
                                &notebook,
                                &ctx,
                                &location_entry,
                                &search_entry,
                                &hidden_toggle,
                                &sidebar_list,
                                &watcher_manager,
                            ),
                            "rename",
                            &err,
                            operations::privileged::Operation::Rename {
                                from: source.clone(),
                                to: source.with_file_name(&name),
                            },
                        );
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
                },
            );
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
        let items_clone = items.clone();

        batch_rename_btn.connect_clicked(move |_| {
            popover.popdown();

            // This used to queue a rename job that renamed every item to
            // *itself* -- a no-op. The dialog is what turns a pattern into
            // real new names: it previews each one, checks for clashes, and
            // only hands back the (old path, new path) pairs once they're
            // fine, which is what the rename job then carries out.
            let named: Vec<(String, PathBuf)> = items_clone
                .iter()
                .map(|item| (item.name(), item.get_path()))
                .collect();

            let on_apply = {
                let window = window.clone();
                let notebook = notebook.clone();
                let ctx = ctx.clone();
                let location_entry = location_entry.clone();
                let search_entry = search_entry.clone();
                let hidden_toggle = hidden_toggle.clone();
                let sidebar_list = sidebar_list.clone();
                let watcher_manager = watcher_manager.clone();

                move |renames: Vec<(PathBuf, PathBuf)>| {
                    start_batch_rename_job_ui(
                        &window,
                        &notebook,
                        &ctx,
                        &location_entry,
                        &search_entry,
                        &hidden_toggle,
                        &sidebar_list,
                        &watcher_manager,
                        renames,
                    );
                }
            };

            ui::batch_rename::show(&window, named, on_apply);
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
            let paths_to_trash = paths.clone();
            start_trash_job_ui(
                &window,
                &notebook,
                &ctx,
                &location_entry,
                &search_entry,
                &hidden_toggle,
                &sidebar_list,
                &watcher_manager,
                paths_to_trash,
            );
        });
    }

    {
        let popover = popover.clone();
        let window = window.clone();
        let items_for_properties = items.clone();

        properties_btn.connect_clicked(move |_| {
            popover.popdown();

            match items_for_properties.as_slice() {
                [item] => crate::ui::properties::show(&window, item),
                many => crate::ui::properties::show_selection(&window, many),
            }
        });
    }

    popover.popup();
}

/// Right-click menu for the split pane's selection.
fn show_split_context_menu(
    job_ui: &JobUi,
    split_pane: &Rc<ui::split_pane::SplitPane>,
    paths: Vec<PathBuf>,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(&split_pane.grid);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let open_btn = Button::with_label("Open");
    let copy_btn = Button::with_label("Copy to Current Folder");
    let move_btn = Button::with_label("Move to Current Folder");

    open_btn.set_sensitive(paths.len() == 1);

    menu_box.append(&open_btn);
    menu_box.append(&copy_btn);
    menu_box.append(&move_btn);

    popover.set_child(Some(&menu_box));

    // A popover attached with `set_parent` stays a child of the grid until
    // it is unparented, so this one unparents itself as soon as it closes
    // (instead of piling up as one more child per right-click), and the
    // buttons' handlers only hold a *weak* reference back to it -- a strong
    // one would be a cycle (popover -> button -> handler -> popover) that
    // nothing could ever free.
    let popover_weak = popover.downgrade();
    popover.connect_closed(|popover| popover.unparent());

    {
        let popover_weak = popover_weak.clone();
        let split_pane = split_pane.clone();
        let path = paths.first().cloned();

        open_btn.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }

            let Some(path) = path.clone() else {
                return;
            };

            if path.is_dir() {
                split_pane.navigate(path);
            } else {
                open_file_default(&path);
            }
        });
    }

    for (button, operation) in [(copy_btn, PendingOp::Copy), (move_btn, PendingOp::Move)] {
        let popover_weak = popover_weak.clone();
        let job_ui = job_ui.clone();
        let paths = paths.clone();

        button.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }

            if let Some((tab_state, _, _, _)) = get_active_widgets(&job_ui.notebook) {
                let destination = tab_state.borrow().current.clone();
                run_quick_transfer(job_ui.clone(), operation, paths.clone(), destination);
            }
        });
    }

    popover.popup();
}

fn typeahead_select(
    ch: char,
    _grid: &gtk::GridView,
    store: &gio::ListStore,
    selection: &gtk::MultiSelection,
) {
    let q = ch.to_lowercase().to_string();
    let n = store.n_items();
    for i in 0..n {
        if let Some(obj) = store.item(i) {
            if let Some(item_obj) = obj.downcast_ref::<ItemObject>() {
                if item_obj.name().to_lowercase().starts_with(&q) {
                    selection.select_item(i, true);
                    return;
                }
            }
        }
    }
}

fn send_job_notification(window: &ApplicationWindow, title: &str, body: &str) {
    if let Some(app) = window.application() {
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        app.send_notification(None, &notification);
    }
}

/// How many entries the Back / Forward history menu lists.
const HISTORY_MENU_LIMIT: usize = 12;

/// The menu behind a right-click on Back (`forward` = false) or Forward:
/// where it would go, nearest first, each one a jump straight there.
fn show_history_menu(anchor: &Button, forward: bool, job_ui: JobUi) {
    let Some((tab_state, _, _, _)) = get_active_widgets(&job_ui.notebook) else {
        return;
    };

    let entries = if forward {
        tab_state.borrow().history.forward_entries()
    } else {
        tab_state.borrow().history.back_entries()
    };

    if entries.is_empty() {
        return;
    }

    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(anchor);

    let menu_box = GtkBox::new(Orientation::Vertical, 2);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    for (index, path) in entries.iter().take(HISTORY_MENU_LIMIT).enumerate() {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        let button = Button::with_label(&name);
        button.set_has_frame(false);
        button.set_halign(gtk::Align::Fill);
        button.set_tooltip_text(Some(&path.display().to_string()));

        // Weak, so the popover and its buttons aren't kept alive by their own
        // click handlers.
        let popover_weak = popover.downgrade();
        let job_ui = job_ui.clone();

        button.connect_clicked(move |_| {
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }

            let Some((tab_state, _, store, _)) = get_active_widgets(&job_ui.notebook) else {
                return;
            };

            let current = tab_state.borrow().current.clone();

            let target = if forward {
                tab_state
                    .borrow_mut()
                    .history
                    .jump_forward(index + 1, &current)
            } else {
                tab_state
                    .borrow_mut()
                    .history
                    .jump_back(index + 1, &current)
            };

            if let Some(target) = target {
                tab_state.borrow_mut().current = target;

                refresh_tab(
                    &tab_state,
                    &store,
                    &job_ui.ctx,
                    &job_ui.location_entry,
                    &job_ui.search_entry,
                    &job_ui.hidden_toggle,
                    &job_ui.sidebar_list,
                );

                update_watcher(&job_ui.notebook, &job_ui.watcher_manager);
            }
        });

        menu_box.append(&button);
    }

    popover.set_child(Some(&menu_box));

    // A popover attached with `set_parent` stays a child of the button until
    // it's unparented, so this one lets go as soon as it closes.
    popover.connect_closed(|popover| popover.unparent());
    popover.popup();
}

/// Right-click on a "Recent" row: clear the list.
fn show_recent_context_menu(
    sidebar_list: &ListBox,
    location_entry: &Entry,
    ctx: &Rc<RefCell<AppContext>>,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(sidebar_list);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let clear_btn = Button::with_label("Clear Recent Files");
    let sidebar_list = sidebar_list.clone();
    let location_entry = location_entry.clone();
    let ctx = ctx.clone();
    let popover_clone = popover.clone();

    clear_btn.connect_clicked(move |_| {
        popover_clone.popdown();

        let _ = gtk::RecentManager::default().purge_items();

        // The list is written out a moment later, so redo the sidebar now
        // and again shortly after.
        for delay_ms in [0u64, 400] {
            let sidebar_list = sidebar_list.clone();
            let location_entry = location_entry.clone();
            let ctx = ctx.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(delay_ms), move || {
                if let Some(win) =
                    get_obj_data::<_, ApplicationWindow>(&location_entry, "main-window")
                {
                    sidebar::build(&sidebar_list, &ctx.borrow().bookmarks, &win);
                }

                glib::ControlFlow::Break
            });
        }
    });

    menu_box.append(&clear_btn);
    popover.set_child(Some(&menu_box));
    popover.popup();
}

fn show_sidebar_context_menu(
    _window: &ApplicationWindow,
    _notebook: &Notebook,
    ctx: &Rc<RefCell<AppContext>>,
    sidebar_list: &ListBox,
    location_entry: &Entry,
    _search_entry: &SearchEntry,
    _hidden_toggle: &CheckButton,
    path: PathBuf,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(sidebar_list);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

    let menu_box = GtkBox::new(Orientation::Vertical, 6);
    menu_box.set_margin_top(6);
    menu_box.set_margin_bottom(6);
    menu_box.set_margin_start(6);
    menu_box.set_margin_end(6);

    let remove_btn = Button::with_label("Remove Bookmark");
    let ctx_clone = ctx.clone();
    let sidebar_list_clone = sidebar_list.clone();
    let location_entry_clone = location_entry.clone();
    let path_clone = path.clone();
    let popover_clone = popover.clone();

    remove_btn.connect_clicked(move |_| {
        popover_clone.popdown();
        // `bookmarks::remove` also saves the list; the bare `retain` this
        // replaced only dropped the entry from memory, so the bookmark came
        // back the next time MITOS Files started.
        let mut c = ctx_clone.borrow_mut();
        bookmarks::remove(&mut c.bookmarks, &path_clone);
        drop(c);
        if let Some(win) =
            get_obj_data::<_, gtk::ApplicationWindow>(&location_entry_clone, "main-window")
        {
            sidebar::build(&sidebar_list_clone, &ctx_clone.borrow().bookmarks, &win);
        }
    });

    menu_box.append(&remove_btn);
    popover.set_child(Some(&menu_box));
    popover.popup();
}

fn show_default_app_picker(
    window: &ApplicationWindow,
    display_apps: Vec<(String, gio::AppInfo)>,
    mime: &str,
) {
    if display_apps.is_empty() {
        crate::ui::dialogs::show_error(
            window,
            &format!("No applications are available for {}.", mime),
        );
        return;
    }

    let dialog = dialogs::build_dialog(window, "Set Default Application");

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);

    for (name, _) in &display_apps {
        let row = gtk::ListBoxRow::new();
        let label = Label::new(Some(name));
        label.set_halign(gtk::Align::Start);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        label.set_margin_start(8);
        label.set_margin_end(8);
        row.set_child(Some(&label));
        list.append(&row);
    }

    if let Some(first_row) = list.row_at_index(0) {
        list.select_row(Some(&first_row));
    }

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(200)
        .build();
    scrolled.set_child(Some(&list));

    dialog.content.append(&scrolled);

    let cancel_btn = dialogs::dialog_button(&dialog.button_row, "Cancel");
    let accept_btn = dialogs::dialog_button(&dialog.button_row, "Set as Default");
    accept_btn.add_css_class("suggested-action");

    {
        let window = dialog.window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    let mime = mime.to_string();
    let window_for_accept = window.clone();
    let dialog_window = dialog.window.clone();

    accept_btn.connect_clicked(move |_| {
        if let Some(row) = list.selected_row() {
            let index = row.index();
            if index >= 0 {
                if let Some((_, app_info)) = display_apps.get(index as usize) {
                    if let Err(err) = crate::mime::applications::set_default_app(app_info, &mime) {
                        crate::ui::dialogs::show_error(
                            &window_for_accept,
                            &format!("Failed: {err}"),
                        );
                    }
                }
            }
        }
        dialog_window.close();
    });

    dialog.window.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::jobs::{ConflictAction, PasteTask};
    use crate::operations::privileged::{Operation, PasteKind};

    #[test]
    fn only_permission_failures_are_offered_an_administrator_retry() {
        assert!(is_permission_error("Permission denied (os error 13)"));
        assert!(is_permission_error("Operation not permitted (os error 1)"));
        assert!(!is_permission_error(
            "No such file or directory (os error 2)"
        ));
        assert!(!is_permission_error(
            "No space left on device (os error 28)"
        ));
    }

    #[test]
    fn a_failed_delete_can_be_retried_and_a_failed_trash_becomes_a_permanent_delete() {
        let paths = vec![PathBuf::from("/root/secret")];

        let delete = elevated_retry_for(&JobRequest::Delete {
            paths: paths.clone(),
        })
        .expect("delete has an administrator retry");
        assert_eq!(delete.operation, Operation::Delete(paths.clone()));

        let trash = elevated_retry_for(&JobRequest::Trash {
            paths: paths.clone(),
        })
        .expect("trash has an administrator retry");
        assert_eq!(trash.operation, Operation::Delete(paths));
        // The user is told it's permanent -- it isn't "the same thing, as root".
        assert!(trash.prompt.contains("permanently"));
    }

    #[test]
    fn a_paste_is_retried_task_for_task_minus_the_skipped_ones() {
        let task = |name: &str, action| PasteTask {
            source: PathBuf::from(format!("/src/{name}")),
            destination: PathBuf::from(format!("/dest/{name}")),
            action,
        };

        let retry = elevated_retry_for(&JobRequest::Paste {
            operation: PendingOp::Move,
            tasks: vec![
                task("a", ConflictAction::KeepBoth),
                task("b", ConflictAction::Replace),
                task("c", ConflictAction::Skip),
            ],
        })
        .expect("paste has an administrator retry");

        match retry.operation {
            Operation::Paste { kind, entries } => {
                assert_eq!(kind, PasteKind::Move);
                assert_eq!(entries.len(), 2);
                assert!(!entries[0].replace);
                assert!(entries[1].replace);
                assert_eq!(entries[1].target, PathBuf::from("/dest/b"));
            }
            other => panic!("unexpected operation: {other:?}"),
        }
    }

    #[test]
    fn jobs_where_root_would_not_help_have_no_retry() {
        assert!(elevated_retry_for(&JobRequest::BatchRename {
            renames: Vec::new()
        })
        .is_none());
        assert!(elevated_retry_for(&JobRequest::ExtractArchive {
            archive_path: PathBuf::from("/a.zip"),
            destination_dir: PathBuf::from("/b"),
        })
        .is_none());
    }

    #[test]
    fn permission_denied_is_recognised_inside_file_manager_errors() {
        let denied =
            error::FileManagerError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let missing =
            error::FileManagerError::Io(std::io::Error::from(std::io::ErrorKind::NotFound));

        assert!(is_permission_denied(&denied));
        assert!(!is_permission_denied(&missing));
        assert!(!is_permission_denied(&error::FileManagerError::InvalidName));
    }

    #[test]
    fn the_trash_folder_is_recognised_by_its_path() {
        // (No user data directory at all -- HOME unset -- means no trash to
        // recognise, which is fine.)
        if let Some(data) = dirs::data_dir() {
            assert!(is_trash_files_dir(&data.join("Trash/files")));
        }

        assert!(!is_trash_files_dir(Path::new("/tmp")));
    }
}
