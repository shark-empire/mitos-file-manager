use gtk::prelude::*;
use gtk::{Label, ListBox, ListBoxRow, Separator};

use crate::navigation::bookmarks::Bookmark;
use crate::navigation::locations;
use std::path::PathBuf;

pub fn build(list: &ListBox, bookmarks: &[Bookmark]) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    // 1. Default Places
    for (label, path) in locations::default_places() {
        let row = ListBoxRow::new();
        let label_widget = Label::new(Some(&label));
        label_widget.set_halign(gtk::Align::Start);
        label_widget.set_margin_start(6);

        row.set_child(Some(&label_widget));
        row.set_tooltip_text(Some(&path.to_string_lossy()));
        row.set_name(&format!("place:{}", path.to_string_lossy()));
        list.append(&row);
    }

    // 2. Separator
    let sep_row = ListBoxRow::new();
    sep_row.set_activatable(false);
    sep_row.set_selectable(false);
    let sep = Separator::new(gtk::Orientation::Horizontal);
    sep.set_margin_top(6);
    sep.set_margin_bottom(6);
    sep_row.set_child(Some(&sep));
    list.append(&sep_row);

    // 3. Bookmarks Label
    let bm_label_row = ListBoxRow::new();
    bm_label_row.set_activatable(false);
    bm_label_row.set_selectable(false);
    let bm_label = Label::new(None);
    bm_label.set_markup("<b>Bookmarks</b>");
    bm_label.set_halign(gtk::Align::Start);
    bm_label.set_margin_start(6);
    bm_label_row.set_child(Some(&bm_label));
    list.append(&bm_label_row);

    // 4. Bookmarks List
    for bm in bookmarks {
        let row = ListBoxRow::new();
        let label_widget = Label::new(Some(&bm.name));
        label_widget.set_halign(gtk::Align::Start);
        label_widget.set_margin_start(6);
        label_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);

        row.set_child(Some(&label_widget));
        row.set_tooltip_text(Some(&bm.path.to_string_lossy()));
        row.set_name(&format!("bm:{}", bm.path.to_string_lossy()));
        list.append(&row);
    }
}

pub fn resolve_click(row: &ListBoxRow) -> Option<PathBuf> {
    if let Some(name) = row.name() {
        if let Some(path_str) = name.strip_prefix("place:") {
            return Some(PathBuf::from(path_str));
        }
        if let Some(path_str) = name.strip_prefix("bm:") {
            return Some(PathBuf::from(path_str));
        }
    }
    None
}
