use crate::filesystem::access;
use crate::filesystem::metadata;
use crate::filesystem::traversal::{self, FolderSize};
use crate::ui::file_list;
use crate::ui::item_object::ItemObject;
use gtk::glib;
use gtk::prelude::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub fn show(parent: &gtk::ApplicationWindow, item: &ItemObject) {
    let window = gtk::Window::builder()
        .title("Properties")
        .transient_for(parent)
        .modal(true)
        .default_width(460)
        .default_height(540)
        .build();

    // Raised when the window closes, so measuring a huge folder stops
    // instead of grinding on with nobody left to read the answer.
    let cancel = Arc::new(AtomicBool::new(false));

    let notebook = gtk::Notebook::new();

    notebook.append_page(
        &build_general_tab(item, &cancel),
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

    {
        let cancel = cancel.clone();

        window.connect_close_request(move |_| {
            cancel.store(true, Ordering::Relaxed);
            glib::Propagation::Proceed
        });
    }

    window.set_child(Some(&notebook));
    window.present();
}

/// Properties for several items at once: how many of each kind, their
/// combined size (folders are measured in the background, so the dialog
/// opens instantly), and one row per item.
pub fn show_selection(parent: &gtk::ApplicationWindow, items: &[ItemObject]) {
    let title = format!("Properties \u{2014} {} items", items.len());

    let window = gtk::Window::builder()
        .title(title.as_str())
        .transient_for(parent)
        .modal(true)
        .default_width(460)
        .default_height(540)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let folders = items.iter().filter(|item| item.is_dir()).count();
    let files = items.len() - folders;

    let summary = gtk::Label::new(Some(&format!(
        "{} selected: {}, {}",
        items.len(),
        count_noun(files, "file", "files"),
        count_noun(folders, "folder", "folders"),
    )));
    summary.set_halign(gtk::Align::Start);
    summary.add_css_class("heading");

    let total_label = gtk::Label::new(Some("Total size: calculating\u{2026}"));
    total_label.set_halign(gtk::Align::Start);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);

    for item in items {
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&file_list::render_file_row(item)));
        list.append(&row);
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    vbox.append(&summary);
    vbox.append(&total_label);
    vbox.append(&scrolled);

    window.set_child(Some(&vbox));

    let cancel = Arc::new(AtomicBool::new(false));
    let paths: Vec<PathBuf> = items.iter().map(|item| item.get_path()).collect();
    let receiver = traversal::spawn_folder_size_job(paths, cancel.clone());

    glib::MainContext::default().spawn_local(async move {
        if let Ok(total) = receiver.recv().await {
            total_label.set_label(&format!("Total size: {}", describe_size(total)));
        }
    });

    window.connect_close_request(move |_| {
        cancel.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });

    window.present();
}

/// "1 file", "3 files".
fn count_noun(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

/// "1.2 GB (341 files, 22 folders)".
fn describe_size(size: FolderSize) -> String {
    format!(
        "{} ({}, {})",
        metadata::format_size(size.bytes),
        count_noun(size.files as usize, "file", "files"),
        count_noun(size.folders as usize, "folder", "folders"),
    )
}

// ============================================================================
// GENERAL
// ============================================================================

fn build_general_tab(item: &ItemObject, cancel: &Arc<AtomicBool>) -> gtk::Widget {
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

    // A folder's own "size" is just the directory entry (the "-" the file
    // views show); what people mean is everything inside it, which takes a
    // walk of the tree -- so start it in the background and fill the number
    // in when it arrives.
    let is_folder = item.is_dir();

    let size_text = if is_folder {
        "Calculating\u{2026}".to_string()
    } else {
        item.size_str()
    };

    let details = extra_details(&path);

    let mut rows: Vec<(&str, String)> = vec![
        ("Name", item.name()),
        ("Type", item.mime_type()),
        ("Size", size_text),
        ("Location", location),
        ("Modified", item.modified_str()),
        ("Accessed", details.accessed),
        ("Created", details.created),
        ("Owner", details.owner),
        ("Group", details.group),
        (
            "Permissions",
            format!("{} ({})", item.permissions(), details.octal),
        ),
        // What *you* may do -- which the permission string alone can't say.
        ("You can", details.access),
        (
            "Symlink",
            if item.is_symlink() {
                "Yes".into()
            } else {
                "No".into()
            },
        ),
    ];

    if let Some(target) = details.link_target {
        rows.push(("Link target", target));
    }

    let mut size_value: Option<gtk::Label> = None;

    let mut row = 1;
    for (label_text, value) in rows {
        let l = gtk::Label::new(Some(label_text));
        l.set_halign(gtk::Align::Start);
        l.add_css_class("heading");

        let v = gtk::Label::new(Some(&value));
        v.set_halign(gtk::Align::Start);
        v.set_wrap(true);
        v.set_selectable(true);

        if is_folder && label_text == "Size" {
            size_value = Some(v.clone());
        }

        grid.attach(&l, 0, row, 1, 1);
        grid.attach(&v, 1, row, 1, 1);
        row += 1;
    }

    if let Some(size_label) = size_value {
        let receiver = traversal::spawn_folder_size_job(vec![path.clone()], cancel.clone());

        glib::MainContext::default().spawn_local(async move {
            if let Ok(total) = receiver.recv().await {
                size_label.set_label(&describe_size(total));
            }
        });
    }

    grid.upcast::<gtk::Widget>()
}

/// The properties that need more than the listing already knows: who owns
/// the file, when it was last opened and created, where a link points, and
/// what the current user is actually allowed to do with it.
struct ExtraDetails {
    owner: String,
    group: String,
    /// Permission bits in octal ("755", or "4755" with a setuid bit).
    octal: String,
    accessed: String,
    created: String,
    link_target: Option<String>,
    access: String,
}

fn extra_details(path: &Path) -> ExtraDetails {
    let metadata = std::fs::symlink_metadata(path).ok();

    let owner = metadata
        .as_ref()
        .map(|m| access::user_name(m.uid()))
        .unwrap_or_else(|| "-".to_string());

    let group = metadata
        .as_ref()
        .map(|m| access::group_name(m.gid()))
        .unwrap_or_else(|| "-".to_string());

    let octal = metadata
        .as_ref()
        .map(|m| format!("{:o}", m.mode() & 0o7777))
        .unwrap_or_else(|| "-".to_string());

    let accessed = metadata::format_modified(metadata.as_ref().and_then(|m| m.accessed().ok()));

    // Not every filesystem records a creation ("birth") time.
    let created = metadata::format_modified(metadata.as_ref().and_then(|m| m.created().ok()));

    let link_target = std::fs::read_link(path)
        .ok()
        .map(|target| target.display().to_string());

    ExtraDetails {
        owner,
        group,
        octal,
        accessed,
        created,
        link_target,
        access: access::effective_access(path).describe(),
    }
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

            if owner_r.is_active() {
                new_mode |= 0o400;
            }
            if owner_w.is_active() {
                new_mode |= 0o200;
            }
            if owner_x.is_active() {
                new_mode |= 0o100;
            }
            if group_r.is_active() {
                new_mode |= 0o040;
            }
            if group_w.is_active() {
                new_mode |= 0o020;
            }
            if group_x.is_active() {
                new_mode |= 0o010;
            }
            if other_r.is_active() {
                new_mode |= 0o004;
            }
            if other_w.is_active() {
                new_mode |= 0o002;
            }
            if other_x.is_active() {
                new_mode |= 0o001;
            }

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

fn build_open_with_tab(window: &gtk::Window, item: &ItemObject) -> gtk::Widget {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;
    use std::fs;

    #[test]
    fn details_report_owner_mode_and_access() {
        let dir = scratch_dir("props-details");
        let file = dir.join("script.sh");
        fs::write(&file, "#!/bin/sh").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();

        let details = extra_details(&file);

        assert_eq!(details.octal, "640");
        assert!(!details.owner.is_empty() && details.owner != "-");
        assert!(!details.group.is_empty() && details.group != "-");
        assert!(details.access.contains("read"));
        assert!(details.link_target.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_shows_where_it_points() {
        let dir = scratch_dir("props-link");
        let target = dir.join("real.txt");
        fs::write(&target, "x").unwrap();
        let link = dir.join("alias");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            extra_details(&link).link_target,
            Some(target.display().to_string())
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_path_degrades_to_placeholders() {
        let details = extra_details(Path::new("/no/such/file"));

        assert_eq!(details.owner, "-");
        assert_eq!(details.octal, "-");
        assert_eq!(details.access, "no access");
    }

    #[test]
    fn counts_are_worded_correctly() {
        assert_eq!(count_noun(1, "file", "files"), "1 file");
        assert_eq!(count_noun(0, "file", "files"), "0 files");
        assert_eq!(count_noun(7, "folder", "folders"), "7 folders");
    }
}
