use gtk::prelude::*;
use gtk::{Label, ListBox, ListBoxRow};

use crate::navigation::locations;
use std::path::PathBuf;

pub fn build(list: &ListBox) {
    for (label, _) in locations::default_places() {
        let row = ListBoxRow::new();

        let label_widget = Label::new(Some(&label));
        label_widget.set_halign(gtk::Align::Start);

        row.set_child(Some(&label_widget));
        list.append(&row);
    }
}

pub fn place_at(index: usize) -> Option<PathBuf> {
    locations::default_places()
        .into_iter()
        .nth(index)
        .map(|(_, path)| path)
}
