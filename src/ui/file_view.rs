use gtk::prelude::*;
use gtk::{Label, ListBox, ListBoxRow};

use crate::filesystem::directory::Item;
use crate::filesystem::metadata;

pub fn clear(list: &ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}

pub fn render(list: &ListBox, items: &[Item]) {
    for item in items {
        let row = ListBoxRow::new();

        let kind = if item.is_dir { "[dir]" } else { "[file]" };

        let size = if item.is_dir {
            "-".to_string()
        } else {
            metadata::format_size(item.metadata.size)
        };

        let modified = metadata::format_modified(item.metadata.modified);
        let permissions = &item.metadata.permissions;

        let text = format!(
            "{kind} {} | {} | {} | {}",
            item.name, size, modified, permissions
        );

        let label = Label::new(Some(&text));
        label.set_halign(gtk::Align::Start);

        row.set_child(Some(&label));
        list.append(&row);
    }
}

pub fn selected_item(list: &ListBox, items: &[Item]) -> Option<Item> {
    let row = list.selected_row()?;
    let index = row.index();

    if index < 0 {
        return None;
    }

    items.get(index as usize).cloned()
}
