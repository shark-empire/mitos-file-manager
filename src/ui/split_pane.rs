use crate::filesystem::directory;
use crate::ui::grid_view;
use crate::ui::item_object::ItemObject;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, GridView, Label, Orientation, ScrolledWindow};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// How many folders the pane's own Back button remembers.
const MAX_HISTORY: usize = 100;

pub struct SplitPaneState {
    pub current: PathBuf,
    pub history: Vec<PathBuf>,
    /// Bumped by every refresh: a listing that finishes after a newer one
    /// started is for a folder the pane has already left, and is dropped.
    pub load_generation: u64,
}

impl SplitPaneState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            current: path,
            history: Vec::new(),
            load_generation: 0,
        }
    }
}

pub struct SplitPane {
    pub container: GtkBox,
    pub location_label: Label,
    pub grid: GridView,
    pub store: gio::ListStore,
    pub selection: gtk::MultiSelection,
    pub state: Rc<RefCell<SplitPaneState>>,
}

impl SplitPane {
    /// Paths of whatever is selected in this pane's grid, in display order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        (0..self.store.n_items())
            .filter(|&position| self.selection.is_selected(position))
            .filter_map(|position| self.store.item(position).and_downcast::<ItemObject>())
            .map(|item| item.get_path())
            .collect()
    }

    /// Point this pane at `path`, remembering where it was so the pane's own
    /// Back button still works.
    pub fn navigate(&self, path: PathBuf) {
        navigate_to(&self.state, &self.store, &self.location_label, path);
    }

    /// Move keyboard focus into the pane's file grid.
    pub fn focus_grid(&self) {
        self.grid.grab_focus();
    }
}

/// Remember `old` as somewhere the pane's Back button can return to.
fn remember(state: &Rc<RefCell<SplitPaneState>>, old: PathBuf) {
    let mut s = state.borrow_mut();

    s.history.push(old);

    if s.history.len() > MAX_HISTORY {
        s.history.remove(0);
    }
}

pub fn build(initial_path: PathBuf) -> SplitPane {
    let container = GtkBox::new(Orientation::Vertical, 4);
    container.set_width_request(350);
    container.set_margin_start(4);
    container.set_margin_end(4);

    // Navigation bar
    let nav_bar = GtkBox::new(Orientation::Horizontal, 4);

    let back_btn = Button::with_label("←");
    let up_btn = Button::with_label("↑");
    let location_label = Label::new(Some(&initial_path.display().to_string()));
    location_label.set_hexpand(true);
    location_label.set_halign(gtk::Align::Start);
    location_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

    nav_bar.append(&back_btn);
    nav_bar.append(&up_btn);
    nav_bar.append(&location_label);

    container.append(&nav_bar);

    // File grid
    let (store, selection) = grid_view::create_model();
    let grid = grid_view::create_grid_view(&selection);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&grid));
    scrolled.set_vexpand(true);

    container.append(&scrolled);

    let state = Rc::new(RefCell::new(SplitPaneState::new(initial_path.clone())));

    // Wire navigation buttons
    {
        let state = state.clone();
        let store = store.clone();
        let location_label = location_label.clone();

        back_btn.connect_clicked(move |_| {
            let prev = {
                let mut s = state.borrow_mut();
                s.history.pop()
            };

            if let Some(prev) = prev {
                state.borrow_mut().current = prev.clone();
                refresh_pane(&state, &store, &location_label);
            }
        });
    }

    {
        let state = state.clone();
        let store = store.clone();
        let location_label = location_label.clone();

        up_btn.connect_clicked(move |_| {
            let parent = {
                let s = state.borrow();
                s.current.parent().map(|p| p.to_path_buf())
            };

            if let Some(parent) = parent {
                let old = state.borrow().current.clone();
                remember(&state, old);
                state.borrow_mut().current = parent;
                refresh_pane(&state, &store, &location_label);
            }
        });
    }

    // Double-click to navigate
    {
        let state = state.clone();
        let store = store.clone();
        let location_label = location_label.clone();

        grid.connect_activate(move |_, pos| {
            // The row's file comes straight from the store: the pane keeps
            // no second copy of the listing.
            let Some(item) = store.item(pos).and_downcast::<ItemObject>() else {
                return;
            };

            let path = item.get_path();

            if item.is_dir() {
                let old = state.borrow().current.clone();
                remember(&state, old);
                state.borrow_mut().current = path;
                refresh_pane(&state, &store, &location_label);
            } else {
                // Files open in their default application, same as in
                // the main view.
                let uri = gio::File::for_path(&path).uri();
                let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
                crate::navigation::recent::record(&path);
            }
        });
    }

    // Initial load
    {
        let state = state.clone();
        let store = store.clone();
        let location_label = location_label.clone();
        refresh_pane(&state, &store, &location_label);
    }

    SplitPane {
        container,
        location_label,
        grid,
        store,
        selection,
        state,
    }
}

/// Show the pane's current folder. Like the main view, the listing is read
/// on a worker thread and poured into the grid in chunks, so a huge folder
/// doesn't freeze the window.
pub fn refresh_pane(
    state: &Rc<RefCell<SplitPaneState>>,
    store: &gio::ListStore,
    location_label: &Label,
) {
    let (current, generation) = {
        let mut s = state.borrow_mut();
        s.load_generation += 1;

        (s.current.clone(), s.load_generation)
    };

    location_label.set_label(&current.display().to_string());

    let (sender, receiver) = async_channel::bounded::<Vec<directory::Item>>(1);

    std::thread::spawn(move || {
        let _ = sender.send_blocking(directory::read_items(&current, false));
    });

    let state = state.clone();
    let store = store.clone();

    glib::MainContext::default().spawn_local(async move {
        let Ok(items) = receiver.recv().await else {
            return;
        };

        // The pane moved on while this was loading.
        if state.borrow().load_generation != generation {
            return;
        }

        let still_current = {
            let state = state.clone();
            move || state.borrow().load_generation == generation
        };

        grid_view::render_progressive(&store, items, still_current);
    });
}

pub fn navigate_to(
    state: &Rc<RefCell<SplitPaneState>>,
    store: &gio::ListStore,
    location_label: &Label,
    path: PathBuf,
) {
    if !path.is_dir() {
        return;
    }

    let old = state.borrow().current.clone();

    if old != path {
        remember(state, old);
        state.borrow_mut().current = path;
        refresh_pane(state, store, location_label);
    }
}
