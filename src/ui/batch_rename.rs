use crate::operations::batch_rename;
use crate::ui::dialogs;
use gtk::prelude::*;
use gtk::{
    ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, SelectionMode, SpinButton,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub fn show<F>(parent: &ApplicationWindow, items: Vec<(String, PathBuf)>, on_apply: F)
where
    F: Fn(Vec<(PathBuf, PathBuf)>) + 'static,
{
    let window = gtk::Window::builder()
        .title("Batch Rename")
        .transient_for(parent)
        .default_width(560)
        .default_height(520)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 8);

    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    // Pattern row
    let pattern_box = GtkBox::new(Orientation::Horizontal, 8);

    let pattern_label = Label::new(Some("Pattern:"));
    let pattern_entry = Entry::new();
    pattern_entry.set_text("Photo_{000}");
    pattern_entry.set_hexpand(true);

    pattern_box.append(&pattern_label);
    pattern_box.append(&pattern_entry);

    // Start number row
    let number_box = GtkBox::new(Orientation::Horizontal, 8);

    let number_label = Label::new(Some("Start at:"));
    let number_spin = SpinButton::with_range(0.0, 999999.0, 1.0);
    number_spin.set_value(1.0);

    number_box.append(&number_label);
    number_box.append(&number_spin);

    // Help text
    let help_label = Label::new(Some(
        "Tokens:  {name}  {ext}  {n}  {000}  {parent}  {date}  {time}",
    ));
    help_label.set_halign(gtk::Align::Start);
    help_label.set_wrap(true);

    // Preview list
    let preview_list = ListBox::new();
    preview_list.set_selection_mode(SelectionMode::None);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&preview_list));
    scrolled.set_vexpand(true);

    // Buttons
    let button_box = GtkBox::new(Orientation::Horizontal, 8);

    let cancel_btn = Button::with_label("Cancel");
    let apply_btn = Button::with_label("Apply Rename");

    apply_btn.add_css_class("suggested-action");

    button_box.append(&cancel_btn);
    button_box.append(&apply_btn);

    root.append(&pattern_box);
    root.append(&number_box);
    root.append(&help_label);
    root.append(&scrolled);
    root.append(&button_box);

    window.set_child(Some(&root));

    // Capture items for the preview and apply closures.
    let items = Rc::new(items);

    // Populate the preview.
    let populate_preview = {
        let items = items.clone();
        let preview_list = preview_list.clone();
        let pattern_entry = pattern_entry.clone();
        let number_spin = number_spin.clone();

        move || {
            while let Some(row) = preview_list.row_at_index(0) {
                preview_list.remove(&row);
            }

            let pattern = pattern_entry.text().to_string();
            let start_number = number_spin.value() as u64;

            let parent_dir = items
                .first()
                .and_then(|(_, path)| path.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));

            let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();

            for (index, (name, path)) in items.iter().enumerate() {
                let new_name = batch_rename::compute_new_name(
                    &pattern,
                    index,
                    start_number,
                    name,
                    &parent_dir,
                );

                let new_path = parent_dir.join(&new_name);

                renames.push((path.clone(), new_path.clone()));

                let row = ListBoxRow::new();
                let row_box = GtkBox::new(Orientation::Horizontal, 6);

                row_box.set_margin_top(2);
                row_box.set_margin_bottom(2);
                row_box.set_margin_start(6);
                row_box.set_margin_end(6);

                let text = format!("{}  →  {}", name, new_name);
                let label = Label::new(Some(&text));
                label.set_halign(gtk::Align::Start);
                label.set_wrap(true);

                row_box.append(&label);
                row.set_child(Some(&row_box));
                preview_list.append(&row);
            }

            // Show warnings if any.
            let warnings = batch_rename::validate_renames(&renames);

            if !warnings.is_empty() {
                let warning_text = warnings.join("\n");

                let row = ListBoxRow::new();
                let label = Label::new(Some(&format!("⚠ {}", warning_text)));
                label.set_wrap(true);
                label.set_halign(gtk::Align::Start);

                row.set_child(Some(&label));
                preview_list.append(&row);
            }
        }
    };

    let populate = Rc::new(populate_preview);

    // Initial preview.
    populate();

    // Update preview when pattern changes.
    {
        let populate = populate.clone();

        pattern_entry.connect_changed(move |_| {
            populate();
        });
    }

    // Update preview when start number changes.
    {
        let populate = populate.clone();

        number_spin.connect_value_changed(move |_| {
            populate();
        });
    }

    // Cancel button.
    {
        let window = window.clone();

        cancel_btn.connect_clicked(move |_| {
            window.close();
        });
    }

    // Apply button.
    {
        let window = window.clone();
        let items = items.clone();
        let pattern_entry = pattern_entry.clone();
        let number_spin = number_spin.clone();
        let parent_window = parent.clone();

        apply_btn.connect_clicked(move |_| {
            let pattern = pattern_entry.text().to_string();
            let start_number = number_spin.value() as u64;

            let parent_dir = items
                .first()
                .and_then(|(_, path)| path.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));

            let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();

            for (index, (name, path)) in items.iter().enumerate() {
                let new_name = batch_rename::compute_new_name(
                    &pattern,
                    index,
                    start_number,
                    name,
                    &parent_dir,
                );

                if new_name.is_empty() {
                    dialogs::show_error(&parent_window, "Pattern produced an empty filename.");
                    return;
                }

                let new_path = parent_dir.join(&new_name);
                renames.push((path.clone(), new_path));
            }

            let warnings = batch_rename::validate_renames(&renames);

            if !warnings.is_empty() {
                dialogs::show_error(
                    &parent_window,
                    &format!("Cannot rename:\n\n{}", warnings.join("\n")),
                );
                return;
            }

            on_apply(renames);
            window.close();
        });
    }

    window.present();
}
