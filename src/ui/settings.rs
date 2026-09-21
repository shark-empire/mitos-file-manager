use crate::config::settings;
use crate::ui::theme::ThemeMode;
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

    // Default view (grid vs list)
    let view_label = Label::new(Some("Default view"));
    view_label.set_halign(gtk::Align::Start);
    view_label.set_hexpand(true);

    let view_dropdown = gtk::DropDown::from_strings(&["Grid", "List"]);

    let current_view = if current.default_view == "list" { 1 } else { 0 };
    view_dropdown.set_selected(current_view);
    view_dropdown.set_halign(gtk::Align::End);

    grid.attach(&view_label, 0, 5, 1, 1);
    grid.attach(&view_dropdown, 1, 5, 1, 1);

    // Recommended default apps (mpv/Celluloid for video+audio, GNOME Text
    // Editor for text) -- a bulk alternative to setting each one by hand
    // through a file's Properties > Open With tab.
    let defaults_section = GtkBox::new(Orientation::Vertical, 6);
    defaults_section.set_margin_top(4);

    let defaults_label = Label::new(Some("Default apps"));
    defaults_label.set_halign(gtk::Align::Start);
    defaults_label.add_css_class("heading");

    let defaults_desc = Label::new(Some(
        "Sets mpv (or Celluloid) as default for video and audio files, and \
         GNOME Text Editor for text/code files -- for any that are installed.",
    ));
    defaults_desc.set_halign(gtk::Align::Start);
    defaults_desc.set_wrap(true);
    defaults_desc.add_css_class("dim-label");

    let defaults_btn = Button::with_label("Set Recommended Defaults");
    defaults_btn.set_halign(gtk::Align::Start);

    let defaults_status = Label::new(None);
    defaults_status.set_halign(gtk::Align::Start);
    defaults_status.set_wrap(true);

    defaults_section.append(&defaults_label);
    defaults_section.append(&defaults_desc);
    defaults_section.append(&defaults_btn);
    defaults_section.append(&defaults_status);

    {
        let defaults_status = defaults_status.clone();

        defaults_btn.connect_clicked(move |_| {
            let outcomes = crate::mime::applications::apply_recommended_defaults();

            let summary = outcomes
                .iter()
                .map(|outcome| match &outcome.app_name {
                    Some(name) if outcome.applied > 0 && outcome.failed == 0 => {
                        format!("{}: {name} ({} types)", outcome.label, outcome.applied)
                    }
                    // Some types took and some didn't -- say so instead of
                    // reporting a clean success.
                    Some(name) if outcome.applied > 0 => format!(
                        "{}: {name} ({} types set, {} couldn't be changed)",
                        outcome.label, outcome.applied, outcome.failed
                    ),
                    Some(name) => format!("{}: found {name}, but couldn't set it", outcome.label),
                    None => format!("{}: no matching app installed, skipped", outcome.label),
                })
                .collect::<Vec<_>>()
                .join("\n");

            defaults_status.set_label(&summary);
        });
    }

    // Buttons
    let button_box = GtkBox::new(Orientation::Horizontal, 8);

    let cancel_btn = Button::with_label("Cancel");
    let apply_btn = Button::with_label("Apply");

    apply_btn.add_css_class("suggested-action");

    button_box.append(&cancel_btn);
    button_box.append(&apply_btn);

    root.append(&grid);
    root.append(&defaults_section);
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
            let theme_mode = if theme_dropdown.selected() == 1 {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            let theme = theme_mode.as_str();

            let view = if view_dropdown.selected() == 1 {
                "list"
            } else {
                "grid"
            };

            settings::apply_and_save(
                hidden_switch.is_active(),
                thumbnails_switch.is_active(),
                max_spin.value() as u64,
                confirm_switch.is_active(),
                theme,
                view,
            );

            apply.as_ref()();

            window.close();
        });
    }

    window.present();
}
