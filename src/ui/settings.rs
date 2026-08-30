use crate::config::settings;
use gtk::prelude::*;
use gtk::{ApplicationWindow, Box as GtkBox, Button, Grid, Label, Orientation, SpinButton, Switch};
use std::rc::Rc;

pub fn show(parent: &ApplicationWindow, apply_changes: Rc<dyn Fn()>) {
    let window = gtk::Window::builder()
        .title("MITOS Files Settings")
        .transient_for(parent)
        .default_width(460)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 10);

    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let grid = Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(10);

    let current = settings::current();

    // Show hidden files
    let hidden_label = Label::new(Some("Show hidden files by default"));
    hidden_label.set_halign(gtk::Align::Start);
    hidden_label.set_hexpand(true);

    let hidden_switch = Switch::new();
    hidden_switch.set_active(current.show_hidden_files);
    hidden_switch.set_halign(gtk::Align::End);

    grid.attach(&hidden_label, 0, 0, 1, 1);
    grid.attach(&hidden_switch, 1, 0, 1, 1);

    // Thumbnails enabled
    let thumbnails_label = Label::new(Some("Enable thumbnails"));
    thumbnails_label.set_halign(gtk::Align::Start);
    thumbnails_label.set_hexpand(true);

    let thumbnails_switch = Switch::new();
    thumbnails_switch.set_active(current.enable_thumbnails);
    thumbnails_switch.set_halign(gtk::Align::End);

    grid.attach(&thumbnails_label, 0, 1, 1, 1);
    grid.attach(&thumbnails_switch, 1, 1, 1, 1);

    // Max thumbnail size
    let max_label = Label::new(Some("Max thumbnail image size (MB)"));
    max_label.set_halign(gtk::Align::Start);
    max_label.set_hexpand(true);

    let max_spin = SpinButton::with_range(1.0, 2048.0, 1.0);
    max_spin.set_value(current.thumbnail_max_mb as f64);
    max_spin.set_halign(gtk::Align::End);

    grid.attach(&max_label, 0, 2, 1, 1);
    grid.attach(&max_spin, 1, 2, 1, 1);

    // Confirm trash
    let confirm_label = Label::new(Some("Confirm before emptying trash"));
    confirm_label.set_halign(gtk::Align::Start);
    confirm_label.set_hexpand(true);

    let confirm_switch = Switch::new();
    confirm_switch.set_active(current.confirm_trash);
    confirm_switch.set_halign(gtk::Align::End);

    grid.attach(&confirm_label, 0, 3, 1, 1);
    grid.attach(&confirm_switch, 1, 3, 1, 1);

    // Theme mode
    let theme_label = Label::new(Some("Theme"));
    theme_label.set_halign(gtk::Align::Start);
    theme_label.set_hexpand(true);

    let theme_dropdown = gtk::DropDown::from_strings(&["Light", "Dark"]);

    let current_theme = if current.theme_mode == "dark" { 1 } else { 0 };
    theme_dropdown.set_selected(current_theme);
    theme_dropdown.set_halign(gtk::Align::End);

    grid.attach(&theme_label, 0, 4, 1, 1);
    grid.attach(&theme_dropdown, 1, 4, 1, 1);

    // Buttons
    let button_box = GtkBox::new(Orientation::Horizontal, 8);

    let cancel_btn = Button::with_label("Cancel");
    let apply_btn = Button::with_label("Apply");

    apply_btn.add_css_class("suggested-action");

    button_box.append(&cancel_btn);
    button_box.append(&apply_btn);

    root.append(&grid);
    root.append(&button_box);

    window.set_child(Some(&root));

    {
        let window = window.clone();

        cancel_btn.connect_clicked(move |_| {
            window.close();
        });
    }

    {
        let window = window.clone();
        let apply = apply_changes.clone();

        apply_btn.connect_clicked(move |_| {
            let theme = if theme_dropdown.selected() == 1 {
                "dark"
            } else {
                "light"
            };

            settings::apply_and_save(
                hidden_switch.is_active(),
                thumbnails_switch.is_active(),
                max_spin.value() as u64,
                confirm_switch.is_active(),
                theme,
            );

            apply.as_ref()();

            window.close();
        });
    }

    window.present();
}
