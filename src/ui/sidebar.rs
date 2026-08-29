use crate::navigation::bookmarks::Bookmark;
use crate::ui::dialogs;
use gtk::gio;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, Image, Label, ListBox, ListBoxRow, Orientation, Separator,
};
use std::path::PathBuf;

pub fn build(list: &ListBox, bookmarks: &[Bookmark], window: &gtk::ApplicationWindow) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    // --- 1. PLACES ---
    add_header(list, "Places");
    
    let places = vec![
        ("Home", "user-home-symbolic", dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))),
        ("Desktop", "user-desktop-symbolic", dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("/Desktop"))),
        ("Documents", "folder-documents-symbolic", dirs::document_dir().unwrap_or_else(|| PathBuf::from("/Documents"))),
        ("Downloads", "folder-download-symbolic", dirs::download_dir().unwrap_or_else(|| PathBuf::from("/Downloads"))),
        ("Music", "folder-music-symbolic", dirs::audio_dir().unwrap_or_else(|| PathBuf::from("/Music"))),
        ("Pictures", "folder-pictures-symbolic", dirs::picture_dir().unwrap_or_else(|| PathBuf::from("/Pictures"))),
        ("Videos", "folder-videos-symbolic", dirs::video_dir().unwrap_or_else(|| PathBuf::from("/Videos"))),
        ("Trash", "user-trash-symbolic", dirs::data_dir().unwrap_or_else(|| PathBuf::from("/.local/share")).join("Trash/files")),
    ];

    for (name, icon, path) in places {
        add_row(list, name, icon, path, None, window);
    }

        // --- RECENT FILES ---
    let recents = crate::navigation::recent::recent_files(10);

    if !recents.is_empty() {
        add_header(list, "Recent");

        for path in recents {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());

            add_row(
                list,
                &name,
                "document-open-recent-symbolic",
                path,
                Some("recent:"),
                window,
            );
        }
    }


    // --- 2. BOOKMARKS ---
    if !bookmarks.is_empty() {
        add_header(list, "Bookmarks");
        for bm in bookmarks {
            add_row(list, &bm.name, "folder-bookmark-symbolic", bm.path.clone(), Some("bm:"), window);
        }
    }

    // --- 3. DEVICES & VOLUMES ---
    let monitor = gio::VolumeMonitor::get();
    let mounts = monitor.mounts();

    // Filter out standard system mounts (like /, /boot, /home) to only show external/network drives
    let external_mounts: Vec<_> = mounts
        .into_iter()
        .filter(|m| {
            if let Some(path) = m.root().path() {
                let path_str = path.to_string_lossy();
                // Keep if it's in /media, /mnt, /run/media, or a network scheme (smb, sftp)
                path_str.starts_with("/media")
                    || path_str.starts_with("/mnt")
                    || path_str.starts_with("/run/media")
                    || m.root().uri().starts_with("smb://")
                    || m.root().uri().starts_with("sftp://")
                    || m.root().uri().starts_with("ftp://")
                    || m.can_eject()
            } else {
                false
            }
        })
        .collect();

    if !external_mounts.is_empty() {
        add_header(list, "Devices");
        for mount in external_mounts {
            let name = mount.name();
            let path = mount.root().path().unwrap_or_else(|| PathBuf::from("/"));
            
            let icon_name = if mount.can_eject() {
                "drive-removable-media-symbolic"
            } else {
                "folder-remote-symbolic"
            };

            let row = ListBoxRow::new();
            let row_box = GtkBox::new(Orientation::Horizontal, 6);
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);
            row_box.set_margin_start(6);
            row_box.set_margin_end(6);

            let icon = Image::from_icon_name(icon_name);
            let label = Label::new(Some(&name));
            label.set_hexpand(true);
            label.set_halign(gtk::Align::Start);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);

            row_box.append(&icon);
            row_box.append(&label);

            if mount.can_unmount() || mount.can_eject() {
                let eject_btn = Button::new();
                let eject_icon = Image::from_icon_name("media-eject-symbolic");
                eject_btn.set_child(Some(&eject_icon));
                eject_btn.set_has_frame(false);
                eject_btn.set_tooltip_text(Some("Eject"));
                eject_btn.set_valign(gtk::Align::Center);

                let window_clone = window.clone();
                let mount_clone = mount.clone();

                eject_btn.connect_clicked(move |_| {
                    let window_clone = window_clone.clone();
                    let mount_clone = mount_clone.clone();

                    mount_clone.unmount_with_operation(
                        gio::MountUnmountFlags::NONE,
                        None,
                        gio::Cancellable::NONE,
                        move |result| {
                            if let Err(err) = result {
                                dialogs::show_error(&window_clone, &format!("Failed to eject: {}", err));
                            }
                        },
                    );
                });

                row_box.append(&eject_btn);
            }

            row.set_child(Some(&row_box));
            // We use the "place:" prefix so the main click handler knows to navigate to it
            row.set_name(&format!("place:{}", path.display()));
            list.append(&row);
        }
    }
}

fn add_header(list: &ListBox, text: &str) {
    let row = ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    
    let box_ = GtkBox::new(Orientation::Vertical, 4);
    box_.set_margin_top(8);
    box_.set_margin_start(6);
    
    let label = Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    label.add_css_class("heading"); // You can style this in CSS later
    
    let sep = Separator::new(Orientation::Horizontal);
    
    box_.append(&label);
    box_.append(&sep);
    row.set_child(Some(&box_));
    list.append(&row);
}

fn add_row(
    list: &ListBox,
    name: &str,
    icon_name: &str,
    path: PathBuf,
    prefix: Option<&str>,
    _window: &gtk::ApplicationWindow,
) {
    let row = ListBoxRow::new();
    let row_box = GtkBox::new(Orientation::Horizontal, 6);
    
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(6);
    row_box.set_margin_end(6);

    let icon = Image::from_icon_name(icon_name);
    let label = Label::new(Some(name));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    row_box.append(&icon);
    row_box.append(&label);

    row.set_child(Some(&row_box));
    
    let row_name = if let Some(p) = prefix {
        format!("{}{}", p, path.display())
    } else {
        format!("place:{}", path.display())
    };
    
    row.set_name(&row_name);
    list.append(&row);
}

/// Helper for main.rs to resolve what path was clicked
pub fn resolve_click(row: &ListBoxRow) -> Option<PathBuf> {
    let name = row.name()?;

    if let Some(path_str) = name.strip_prefix("place:") {
        Some(PathBuf::from(path_str))
    } else if let Some(path_str) = name.strip_prefix("bm:") {
        Some(PathBuf::from(path_str))
    } else if let Some(path_str) = name.strip_prefix("recent:") {
        Some(PathBuf::from(path_str))
    } else {
        None
    }
}

