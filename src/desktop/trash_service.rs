//! `org.mitos.Trash` -- the trash can as a D-Bus service, so the desktop
//! shell can show a full/empty trash icon, a settings panel can "Empty
//! Trash", and other apps can list or restore what was deleted, without
//! each of them re-implementing the freedesktop trash layout.
//!
//! Object path `/org/mitos/Trash`. Items are identified by the path of the
//! trashed file itself (the `file_path` in `List`); the service only ever
//! acts on paths it currently lists, so it can't be talked into deleting
//! anything else.

use crate::filesystem::{mounts, trash};
use std::sync::Once;

fn all_items() -> Vec<trash::TrashItem> {
    trash::list(&mounts::removable_roots())
}

/// Run `action` on the trashed item whose file is at `file_path`.
fn with_item(
    file_path: &str,
    action: impl FnOnce(&trash::TrashItem) -> std::io::Result<()>,
) -> Result<(), String> {
    let item = all_items()
        .into_iter()
        .find(|item| item.file_path.to_string_lossy() == file_path)
        .ok_or_else(|| format!("\"{file_path}\" is not in the trash"))?;

    action(&item).map_err(|err| err.to_string())
}

/// Start serving `org.mitos.Trash` on the session bus, once per process.
pub fn start() {
    static STARTED: Once = Once::new();

    STARTED.call_once(|| {
        std::thread::spawn(|| {
            if let Err(err) = run_dbus_service() {
                eprintln!("Trash D-Bus service error: {err}");
            }
        });
    });
}

fn run_dbus_service() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use zbus::blocking::Connection;
    use zbus::interface;

    struct TrashService;

    #[interface(name = "org.mitos.Trash")]
    impl TrashService {
        /// How many items are in the trash.
        fn count(&self) -> u32 {
            all_items().len() as u32
        }

        fn is_empty(&self) -> bool {
            all_items().is_empty()
        }

        /// `(file_path, original_path, deletion_date, location)` for every
        /// trashed item. `deletion_date` is empty when unknown; `location`
        /// is "Home" or the name of the drive the item was deleted from.
        fn list(&self) -> Vec<(String, String, String, String)> {
            all_items()
                .into_iter()
                .map(|item| {
                    (
                        item.file_path.to_string_lossy().to_string(),
                        item.original_path.to_string_lossy().to_string(),
                        item.deletion_date.clone().unwrap_or_default(),
                        item.location_label.clone(),
                    )
                })
                .collect()
        }

        fn restore(&self, file_path: &str) -> zbus::fdo::Result<()> {
            with_item(file_path, trash::restore).map_err(zbus::fdo::Error::Failed)
        }

        fn delete_forever(&self, file_path: &str) -> zbus::fdo::Result<()> {
            with_item(file_path, trash::delete_forever).map_err(zbus::fdo::Error::Failed)
        }

        fn empty(&self) -> zbus::fdo::Result<()> {
            trash::empty(&mounts::removable_roots())
                .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))
        }
    }

    let connection = Connection::session()?;

    connection.request_name("org.mitos.Trash")?;

    connection
        .object_server()
        .at("/org/mitos/Trash", TrashService)?;

    // Requests are served on the connection's own executor; this thread
    // only has to keep the connection alive, and can sleep indefinitely.
    loop {
        std::thread::park();
    }
}
