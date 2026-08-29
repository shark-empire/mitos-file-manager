use crate::ui::item_object::ItemObject;
use gtk::prelude::*;
use std::os::unix::fs::PermissionsExt;

pub fn show(parent: &gtk::ApplicationWindow, item: &ItemObject) {
    let window = gtk::Window::builder()
        .title("Properties")
        .transient_for(parent)
        .modal(true)
        .default_width(460)
        .default_height(540)
        .build();

    let notebook = gtk::Notebook::new();

    notebook.append_page(
        &build_general_tab(item),
        Some(&gtk::Label::new(Some("General"))),
    );

    notebook.append_page(
        &build_permissions_tab(item),
        Some(&gtk::Label::new(Some("Permissions"))),
    );

    notebook.append_page(
        &build_open_with_tab(&window, item),
        Some(&gtk::Label::new(Some("Open With"))),
    );

    window.set_child(Some(&notebook));
    window.present();
}

// ============================================================================
// GENERAL
// ============================================================================

fn build_general_tab(item: &ItemObject) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.set_margin_top(16);
    grid.set_margin_bottom(16);
    grid.set_margin_start(16);
    grid.set_margin_end(16);
    grid.set_row_spacing(8);
    grid.set_column_spacing(16);

    let icon = gtk::Image::from_icon_name(&item.icon_name());
    icon.set_pixel_size(48);
    grid.attach(&icon, 0, 0, 2, 1);

    let path = item.get_path();
    let location = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let rows: Vec<(&str, String)> = vec![
        ("Name", item.name()),
        ("Type", item.mime_type()),
        ("Size", item.size_str()),
        ("Location", location),
        ("Modified", item.modified_str()),
        ("Symlink", if item.is_symlink() { "Yes".into() } else { "No".into() }),
    ];

    let mut row = 1;
    for (label_text, value) in rows {
        let l = gtk::Label::new(Some(label_text));
        l.set_halign(gtk::Align::Start);
        l.add_css_class("heading");

        let v = gtk::Label::new(Some(&value));
        v.set_halign(gtk::Align::Start);
        v.set_wrap(true);
        v.set_selectable(true);

        grid.attach(&l, 0, row, 1, 1);
        grid.attach(&v, 1, row, 1, 1);
        row += 1;
    }

    grid.upcast::<gtk::Widget>()
}

// ============================================================================
// PERMISSIONS
// ============================================================================

fn build_permissions_tab(item: &ItemObject) -> gtk::Widget {
    let path = item.get_path();

    let current_mode = std::fs::symlink_metadata(&path)
        .map(|m| m.permissions().mode())
        .unwrap_or(0o644);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(24);

    // Header row
    for (col, title) in ["Owner", "Group", "Others"].iter().enumerate() {
        let l = gtk::Label::new(Some(title));
        l.add_css_class("heading");
        grid.attach(&l, col as i32 + 1, 0, 1, 1);
    }

    let mk = |active: bool| {
        let cb = gtk::CheckButton::new();
        cb.set_active(active);
        cb.set_halign(gtk::Align::Center);
        cb
    };

    let owner_r = mk(current_mode & 0o400 != 0);
    let owner_w = mk(current_mode & 0o200 != 0);
    let owner_x = mk(current_mode & 0o100 != 0);
    let group_r = mk(current_mode & 0o040 != 0);
    let group_w = mk(current_mode & 0o020 != 0);
    let group_x = mk(current_mode & 0o010 != 0);
    let other_r = mk(current_mode & 0o004 != 0);
    let other_w = mk(current_mode & 0o002 != 0);
    let other_x = mk(current_mode & 0o001 != 0);

    let rows = [
        ("Read", &owner_r, &group_r, &other_r),
        ("Write", &owner_w, &group_w, &other_w),
        ("Execute", &owner_x, &group_x, &other_x),
    ];

    for (row_idx, (label_text, o, g, ot)) in rows.iter().enumerate() {
        let l = gtk::Label::new(Some(label_text));
        l.set_halign(gtk::Align::Start);
        grid.attach(&l, 0, row_idx as i32 + 1, 1, 1);
        grid.attach(*o, 1, row_idx as i32 + 1, 1, 1);
        grid.attach(*g, 2, row_idx as i32 + 1, 1, 1);
        grid.attach(*ot, 3, row_idx as i32 + 1, 1, 1);
    }

    vbox.append(&grid);

    let apply_btn = gtk::Button::with_label("Apply");
    apply_btn.set_halign(gtk::Align::Start);
    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);

    {
        let path = path.clone();
        let owner_r = owner_r.clone();
        let owner_w = owner_w.clone();
        let owner_x = owner_x.clone();
        let group_r = group_r.clone();
        let group_w = group_w.clone();
        let group_x = group_x.clone();
        let other_r = other_r.clone();
        let other_w = other_w.clone();
        let other_x = other_x.clone();
        let status = status.clone();

        apply_btn.connect_clicked(move |_| {
            let mut new_mode: u32 = current_mode & 0o7000;

            if owner_r.is_active() { new_mode |= 0o400; }
            if owner_w.is_active() { new_mode |= 0o200; }
            if owner_x.is_active() { new_mode |= 0o100; }
            if group_r.is_active() { new_mode |= 0o040; }
            if group_w.is_active() { new_mode |= 0o020; }
            if group_x.is_active() { new_mode |= 0o010; }
            if other_r.is_active() { new_mode |= 0o004; }
            if other_w.is_active() { new_mode |= 0o002; }
            if other_x.is_active() { new_mode |= 0o001; }

            match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(new_mode)) {
                Ok(()) => status.set_label("Permissions updated."),
                Err(err) => status.set_label(&format!("Failed: {err}")),
            }
        });
    }

    vbox.append(&apply_btn);
    vbox.append(&status);

    vbox.upcast::<gtk::Widget>()
}

// ============================================================================
// OPEN WITH
// ============================================================================

fn build_open_with_tab(
    window: &gtk::ApplicationWindow,
    item: &ItemObject,
) -> gtk::Widget {
    let mime = item.mime_type();
    let apps = crate::mime::applications::apps_for_mime(&mime);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);

    for app in &apps {
        let row = gtk::ListBoxRow::new();
        let label = gtk::Label::new(Some(&app.display_name()));
        label.set_halign(gtk::Align::Start);
        row.set_child(Some(&label));
        list.append(&row);
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    let set_btn = gtk::Button::with_label("Set as Default");
    set_btn.set_halign(gtk::Align::Start);
    let status = gtk::Label::new(None);
    status.set_halign(gtk::Align::Start);

    {
        let window = window.clone();
        let list = list.clone();
        let apps = apps.clone();
        let mime = mime.clone();
        let status = status.clone();

        set_btn.connect_clicked(move |_| {
            let Some(row) = list.selected_row() else {
                status.set_label("Select an application first.");
                return;
            };

            let index = row.index();
            if index < 0 || index as usize >= apps.len() {
                return;
            }

            match crate::mime::applications::set_default_app(&apps[index as usize], &mime) {
                Ok(()) => status.set_label("Default application updated."),
                Err(err) => {
                    crate::ui::dialogs::show_error(&window, &format!("Failed: {err}"));
                }
            }
        });
    }

    vbox.append(&scrolled);
    vbox.append(&set_btn);
    vbox.append(&status);

    vbox.upcast::<gtk::Widget>()
}
