use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Entry, Orientation, Stack};
use std::path::Path;

pub fn build() -> (GtkBox, Stack, GtkBox, Entry) {
    let outer = GtkBox::new(Orientation::Horizontal, 6);

    let stack = Stack::new();
    stack.set_hexpand(true);

    let crumbs = GtkBox::new(Orientation::Horizontal, 0);
    crumbs.set_hexpand(true);

    let entry = Entry::new();
    entry.set_hexpand(true);

    stack.add_named(&crumbs, "crumbs");
    stack.add_named(&entry, "entry");
    stack.set_visible_child_name("crumbs");

    let edit_btn = Button::with_label("Edit");
    edit_btn.set_has_frame(false);

    {
        let stack = stack.clone();
        let entry = entry.clone();

        edit_btn.connect_clicked(move |btn| {
            if stack.visible_child_name().as_deref() == Some("entry") {
                stack.set_visible_child_name("crumbs");
                btn.set_label("Edit");
            } else {
                stack.set_visible_child_name("entry");
                entry.grab_focus();
                btn.set_label("Done");
            }
        });
    }

    outer.append(&stack);
    outer.append(&edit_btn);

    (outer, stack, crumbs, entry)
}

pub fn update(crumbs: &GtkBox, entry: &Entry, path: &Path) {
    if !entry.has_focus() {
        entry.set_text(&path.display().to_string());
    }

    while let Some(child) = crumbs.first_child() {
        crumbs.remove(&child);
    }

    let mut ancestors = Vec::new();
    let mut current = path.to_path_buf();

    loop {
        ancestors.push(current.clone());

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    ancestors.reverse();

    for ancestor in ancestors {
        let label = if ancestor == Path::new("/") {
            "/".to_string()
        } else {
            ancestor
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string())
        };

        let button = Button::with_label(&label);
        button.set_has_frame(false);

        let entry = entry.clone();
        let target = ancestor.to_string_lossy().to_string();

        button.connect_clicked(move |_| {
            entry.set_text(&target);
            entry.activate();
        });

        crumbs.append(&button);
    }
}
