use crate::navigation::bookmarks::Bookmark;
use crate::navigation::locations;
use crate::ui::dialogs;
use gtk::gio;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Image, Label, ListBox, ListBoxRow, Orientation, Separator};
use std::path::PathBuf;

pub fn build(list: &ListBox, bookmarks: &[Bookmark], window: &gtk::ApplicationWindow) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    // --- 1. PLACES ---
    add_header(list, "Places");

    let home = locations::home_dir();

    for (name, path) in locations::default_places() {
        // Home and the filesystem root are always meaningful. The XDG
        // folders only get a row if they really exist and aren't just
        // $HOME again (what an unset or disabled entry in
        // user-dirs.dirs resolves to) -- a row that goes nowhere when
        // clicked is worse than no row.
        let always_shown = name == "Home" || name == "Computer";

        if !always_shown && (path == home || !path.is_dir()) {
            continue;
        }

        add_row(list, &name, place_icon(&name), path, None, window);
    }

    add_row(
        list,
        "Trash",
        "user-trash-symbolic",
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/.local/share"))
            .join("Trash/files"),
        None,
        window,
    );

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
            add_row(
                list,
                &bm.name,
                "folder-bookmark-symbolic",
                bm.path.clone(),
                Some("bm:"),
                window,
            );
        }
    }

    // --- 3. DEVICES & VOLUMES ---
    add_header(list, "Devices");

    let (network, local): (Vec<gio::Mount>, Vec<gio::Mount>) =
        external_mounts().into_iter().partition(is_network_mount);

    for mount in &local {
        add_mount_row(list, mount, window);
    }

    // Drives that are plugged in but not mounted (automounting is off, or
    // it failed): clicking one mounts it and opens it.
    for volume in unmounted_volumes() {
        add_volume_row(list, &volume);
    }

    // --- 4. NETWORK ---
    add_header(list, "Network");

    for mount in &network {
        add_mount_row(list, mount, window);
    }

    // --- "Connect to Server..." action row, always available so the user
    // can mount a new smb:// / sftp:// / ftp:// location. It's not a place
    // to navigate to, so it gets its own widget-name the click handler
    // checks for before falling through to `resolve_click`.
    {
        let row = ListBoxRow::new();
        let row_box = GtkBox::new(Orientation::Horizontal, 6);
        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(6);
        row_box.set_margin_end(6);

        let icon = Image::from_icon_name("network-server-symbolic");
        let label = Label::new(Some("Connect to Server…"));
        label.set_hexpand(true);
        label.set_halign(gtk::Align::Start);

        row_box.append(&icon);
        row_box.append(&label);
        row.set_child(Some(&row_box));
        row.set_widget_name("action:connect-to-server");
        list.append(&row);
    }
}

/// A mounted drive or share, with an Eject / Unmount button.
fn add_mount_row(list: &ListBox, mount: &gio::Mount, window: &gtk::ApplicationWindow) {
    let name = mount.name();
    let path = mount.root().path().unwrap_or_else(|| PathBuf::from("/"));

    let icon_name = if is_network_mount(mount) {
        "folder-remote-symbolic"
    } else if mount.can_eject() {
        "drive-removable-media-symbolic"
    } else {
        "drive-harddisk-symbolic"
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
        // Drives that can be ejected (USB sticks, optical discs) are
        // ejected -- unmounted *and* powered down, so it's safe to pull the
        // plug. Everything else, network shares included, is just
        // unmounted.
        let ejects = mount.can_eject();

        let eject_btn = Button::new();
        let eject_icon = Image::from_icon_name("media-eject-symbolic");
        eject_btn.set_child(Some(&eject_icon));
        eject_btn.set_has_frame(false);
        eject_btn.set_tooltip_text(Some(if ejects { "Eject" } else { "Unmount" }));
        eject_btn.set_valign(gtk::Align::Center);

        let window_clone = window.clone();
        let mount_clone = mount.clone();

        eject_btn.connect_clicked(move |_| {
            let window_for_result = window_clone.clone();

            let report = move |result: Result<(), gtk::glib::Error>| {
                if let Err(err) = result {
                    dialogs::show_error(
                        &window_for_result,
                        &format!(
                            "Failed to {}: {}",
                            if ejects { "eject" } else { "unmount" },
                            err
                        ),
                    );
                }
            };

            if ejects {
                mount_clone.eject_with_operation(
                    gio::MountUnmountFlags::NONE,
                    None::<&gtk::gio::MountOperation>,
                    gio::Cancellable::NONE,
                    report,
                );
            } else {
                mount_clone.unmount_with_operation(
                    gio::MountUnmountFlags::NONE,
                    None::<&gtk::gio::MountOperation>,
                    gio::Cancellable::NONE,
                    report,
                );
            }
        });

        row_box.append(&eject_btn);
    }

    row.set_child(Some(&row_box));
    // We use the "place:" prefix so the main click handler knows to navigate to it
    row.set_widget_name(&format!("place:{}", path.display()));
    list.append(&row);
}

/// A drive that's present but not mounted.
fn add_volume_row(list: &ListBox, volume: &gio::Volume) {
    let row = ListBoxRow::new();
    let row_box = GtkBox::new(Orientation::Horizontal, 6);
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(6);
    row_box.set_margin_end(6);

    let icon = Image::from_icon_name("drive-removable-media-symbolic");
    let label = Label::new(Some(&format!("{} (not mounted)", volume.name())));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    row_box.append(&icon);
    row_box.append(&label);
    row.set_child(Some(&row_box));
    row.set_tooltip_text(Some("Click to mount"));
    // The click handler finds the volume again from this id.
    row.set_widget_name(&format!("volume:{}", volume_id(volume)));
    list.append(&row);
}

/// A stable identifier for a volume: its UUID, else its device node, else
/// its name.
pub fn volume_id(volume: &gio::Volume) -> String {
    volume
        .uuid()
        .map(|uuid| uuid.to_string())
        .or_else(|| {
            volume
                .identifier("unix-device")
                .map(|device| device.to_string())
        })
        .unwrap_or_else(|| volume.name().to_string())
}

/// Volumes that are present, could be mounted, and would normally be
/// mounted automatically -- but currently aren't.
pub fn unmounted_volumes() -> Vec<gio::Volume> {
    gio::VolumeMonitor::get()
        .volumes()
        .into_iter()
        .filter(|volume| {
            volume.get_mount().is_none() && volume.can_mount() && volume.should_automount()
        })
        .collect()
}

/// Mount `volume` (prompting for a password or the like if it needs one)
/// and, once it's mounted, call `on_mounted` with where it's mounted.
pub fn mount_volume(
    volume: &gio::Volume,
    window: &gtk::ApplicationWindow,
    on_mounted: impl FnOnce(PathBuf) + 'static,
) {
    let operation = gtk::MountOperation::new(Some(window));
    let window_for_error = window.clone();
    let volume_for_result = volume.clone();

    volume.mount(
        gio::MountMountFlags::NONE,
        Some(&operation),
        gio::Cancellable::NONE,
        move |result| match result {
            Ok(()) => {
                if let Some(path) = volume_for_result
                    .get_mount()
                    .and_then(|mount| mount.root().path())
                {
                    on_mounted(path);
                }
            }
            Err(err) => dialogs::show_error(
                &window_for_error,
                &format!("Couldn't mount {}: {}", volume_for_result.name(), err),
            ),
        },
    );
}

/// Is this a network share (smb://, sftp://, ...) rather than a local drive?
pub fn is_network_mount(mount: &gio::Mount) -> bool {
    !mount.root().uri().starts_with("file://")
}

/// A cheap fingerprint of everything the sidebar shows: bookmarks, which
/// user folders exist, mounted and unmounted drives, and the recent-files
/// list. The window compares it before rebuilding, so the sidebar is redone
/// when one of those changes -- not every time a folder is opened.
pub fn signature(bookmarks: &[Bookmark]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    for bookmark in bookmarks {
        bookmark.name.hash(&mut hasher);
        bookmark.path.hash(&mut hasher);
    }

    for (name, path) in locations::default_places() {
        name.hash(&mut hasher);
        path.is_dir().hash(&mut hasher);
    }

    for mount in external_mounts() {
        mount.name().to_string().hash(&mut hasher);
        mount.root().uri().to_string().hash(&mut hasher);
        mount.can_eject().hash(&mut hasher);
    }

    for volume in unmounted_volumes() {
        volume_id(&volume).hash(&mut hasher);
    }

    if let Some(xbel) = crate::navigation::recent::xbel_path() {
        if let Ok(metadata) = std::fs::metadata(xbel) {
            metadata.len().hash(&mut hasher);
            metadata.modified().ok().hash(&mut hasher);
        }
    }

    hasher.finish()
}

/// Sidebar icon for one of `locations::default_places`' entries.
fn place_icon(name: &str) -> &'static str {
    match name {
        "Home" => "user-home-symbolic",
        "Desktop" => "user-desktop-symbolic",
        "Documents" => "folder-documents-symbolic",
        "Downloads" => "folder-download-symbolic",
        "Music" => "folder-music-symbolic",
        "Pictures" => "folder-pictures-symbolic",
        "Videos" => "folder-videos-symbolic",
        "Public" => "folder-publicshare-symbolic",
        "Computer" => "drive-harddisk-symbolic",
        _ => "folder-symbolic",
    }
}

/// Currently-mounted volumes MITOS Files treats as "external" -- shown in
/// the sidebar's Devices section, and (via `trash::list`/`trash::empty`)
/// scanned for their own per-device trash can. Removable media, or
/// anything mounted over a network share.
pub fn external_mounts() -> Vec<gio::Mount> {
    let monitor = gio::VolumeMonitor::get();

    monitor
        .mounts()
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
        .collect()
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

    row.set_widget_name(&row_name);
    list.append(&row);
}

/// Helper for main.rs to resolve what path was clicked
pub fn resolve_click(row: &ListBoxRow) -> Option<PathBuf> {
    let name = row.widget_name();

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
