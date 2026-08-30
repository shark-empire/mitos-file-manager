use gtk::prelude::*;
use gtk::{Box as GtkBox, Image, Label, ListBox, ListBoxRow, Orientation};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct TreeViewState {
    pub expanded: HashSet<PathBuf>,
    pub root: PathBuf,
}

impl TreeViewState {
    pub fn new(root: PathBuf) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());

        Self { expanded, root }
    }
}

pub fn build(root: PathBuf) -> (ListBox, Rc<RefCell<TreeViewState>>) {
    let list = ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);

    let state = Rc::new(RefCell::new(TreeViewState::new(root)));

    render_tree(&state, &list);

    (list, state)
}

pub fn render_tree(state: &Rc<RefCell<TreeViewState>>, list: &ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let root = {
        let s = state.borrow();
        s.root.clone()
    };

    let mut rows: Vec<(PathBuf, usize, bool)> = Vec::new();

    collect_rows(&root, 0, state, &mut rows);

    for (path, depth, is_expanded) in rows {
        let row = create_tree_row(&path, depth, is_expanded);
        list.append(&row);
    }
}

fn collect_rows(
    dir: &Path,
    depth: usize,
    state: &Rc<RefCell<TreeViewState>>,
    rows: &mut Vec<(PathBuf, usize, bool)>,
) {
    let is_expanded = {
        let s = state.borrow();
        s.expanded.contains(dir)
    };

    rows.push((dir.to_path_buf(), depth, is_expanded));

    if !is_expanded {
        return;
    }

    let mut subdirs: Vec<PathBuf> = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                if !name.starts_with('.') {
                    subdirs.push(path);
                }
            }
        }
    }

    subdirs.sort();

    for subdir in subdirs {
        collect_rows(&subdir, depth + 1, state, rows);
    }
}

fn create_tree_row(path: &Path, depth: usize, is_expanded: bool) -> ListBoxRow {
    let row = ListBoxRow::new();

    let row_box = GtkBox::new(Orientation::Horizontal, 4);

    row_box.set_margin_top(2);
    row_box.set_margin_bottom(2);
    row_box.set_margin_start(4);
    row_box.set_margin_end(4);

    // Indentation
    let indent = depth * 16;
    row_box.set_margin_start((4 + indent) as i32);

    // Expand/collapse arrow
    let arrow_icon = if is_expanded {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    };

    let arrow = Image::from_icon_name(arrow_icon);
    arrow.set_pixel_size(12);

    // Folder icon
    let folder_icon = Image::from_icon_name("folder-symbolic");
    folder_icon.set_pixel_size(16);

    // Folder name
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let label = Label::new(Some(&name));
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    row_box.append(&arrow);
    row_box.append(&folder_icon);
    row_box.append(&label);

    row.set_child(Some(&row_box));

    // Store the path in the row name for later retrieval
    row.set_name(&path.display().to_string());

    row
}

pub fn toggle_folder(state: &Rc<RefCell<TreeViewState>>, list: &ListBox, path: PathBuf) {
    {
        let mut s = state.borrow_mut();

        if s.expanded.contains(&path) {
            s.expanded.remove(&path);
        } else {
            s.expanded.insert(path);
        }
    }

    render_tree(state, list);
}
