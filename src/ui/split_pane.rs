use crate::filesystem::directory;
use crate::ui::grid_view;
use gtk::gio;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Entry, GridView, Label, ListBox, Orientation, ScrolledWindow};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub struct SplitPaneState {
    pub current: PathBuf,
    pub history: Vec<PathBuf>,
    pub items: Vec<directory::Item>,
}

impl SplitPaneState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            current: path,
            history: Vec::new(),
            items: Vec::new(),
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
                state.borrow_mut().history.push(old);
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
            let item = {
                let s = state.borrow();
                s.items.get(pos as usize).cloned()
            };

            if let Some(item) = item {
                if item.is_dir {
                    let old = state.borrow().current.clone();
                    state.borrow_mut().history.push(old);
                    state.borrow_mut().current = item.path.clone();
                    refresh_pane(&state, &store, &location_label);
                }
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

pub fn refresh_pane(
    state: &Rc<RefCell<SplitPaneState>>,
    store: &gio::ListStore,
    location_label: &Label,
) {
    let mut s = state.borrow_mut();
    let current = s.current.clone();

    location_label.set_label(&current.display().to_string());

    let items = directory::read_items(&current, false);
    grid_view::render(store, &items);

    s.items = items;
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
        state.borrow_mut().history.push(old);
        state.borrow_mut().current = path;
        refresh_pane(state, store, location_label);
    }
}
