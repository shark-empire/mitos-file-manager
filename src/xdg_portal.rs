//! A real implementation of `org.freedesktop.portal.FileChooser`, the
//! standard XDG Desktop Portal interface sandboxed/Flatpak/Snap apps and
//! portal-aware toolkits (GTK's `FileChooserNative`, etc.) call to show a
//! native file picker. This is what lets MITOS Files act as the system's
//! file-picker backend for *any* app, not just ones that know about MITOS
//! specifically.
//!
//! This is intentionally separate from `service.rs`'s `org.mitos.FilePicker`
//! -- that custom interface still works exactly as before for anything
//! already using it. This file only *adds* an object at the well-known
//! portal path; it doesn't touch the existing one.
//!
//! Spec shape (see the xdg-desktop-portal documentation for the full
//! version): a call to `OpenFile`/`SaveFile` must return almost
//! immediately with an `o` (object path) "request handle", *before* the
//! user has necessarily even seen a dialog -- the actual selection shows
//! up later as a `Response` signal on that same object path. That's
//! different enough from `org.mitos.FilePicker`'s "block until the user
//! answers, then return the result" shape that it needs its own request
//! object per call, created and torn down through the code below.
//!
//! Known simplifications, on purpose, given no D-Bus session or compiler
//! was available to test this against while writing it (see CHANGES.md):
//! - `options` keys `filters` / `current_filter` / `choices` aren't read.
//!   Every call shows MITOS Files' plain picker; a caller-supplied filter
//!   list doesn't narrow it yet.
//! - The request's object-path token is always server-generated. The
//!   spec lets a client suggest one via `options.handle_token` so it can
//!   compute the path itself, but honoring that isn't required for
//!   correctness -- only for a client-side optimization we don't need.
//! - `SaveFiles` (choose a target *folder* for several already-named
//!   files) is now implemented: it shows a folder picker, then joins the
//!   chosen folder with each name from the `files` option to build the
//!   result `uris`, one per input file. If `files` comes through empty,
//!   it falls back to a single "Untitled" entry rather than returning a
//!   folder with nothing in it.
//! - Reading `directory` / `multiple` out of the `options` dict uses
//!   zvariant's `Value`/`OwnedValue` conversions as best-recalled without
//!   a compiler to check against; if `cargo check` flags `option_bool`
//!   below, that's the one function to look at first. `SaveFiles`'s
//!   `current_folder` (`ay`) / `files` (`aay`) reads -- `option_path` and
//!   `option_path_list` below -- carry the exact same caveat: byte
//!   arrays downcasting to `Vec<u8>` / `Vec<Vec<u8>>` is my best recall
//!   of how zvariant represents them, not something this pass could
//!   verify.

use gtk::gio::prelude::FileExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
// Or, use the wildcard: use gtk::gio::prelude::*;

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::service::{PortalRequest, PortalResponse};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Register `org.freedesktop.portal.FileChooser` at the well-known portal
/// path on `connection`, alongside whatever's already registered on it
/// (namely `org.mitos.FilePicker` -- see `service.rs`). `request_tx` is
/// the same channel the GTK main thread already polls for
/// `org.mitos.FilePicker`, reused here so both interfaces show the exact
/// same dialogs through the exact same code path.
pub fn register(
    connection: &Connection,
    request_tx: mpsc::Sender<PortalRequest>,
) -> zbus::Result<()> {
    let portal = FileChooserPortal {
        connection: connection.clone(),
        request_tx,
    };

    connection
        .object_server()
        .at("/org/freedesktop/portal/desktop", portal)?;

    Ok(())
}

struct FileChooserPortal {
    connection: Connection,
    request_tx: mpsc::Sender<PortalRequest>,
}

#[interface(name = "org.freedesktop.portal.FileChooser")]
impl FileChooserPortal {
    #[zbus(name = "OpenFile")]
    fn open_file(
        &self,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let directory = option_bool(&options, "directory");
        let title = title.to_string();
        let request_tx = self.request_tx.clone();

        self.begin_request(move |response_tx| {
            if directory {
                // MITOS Files doesn't have a separate folder-only picker
                // dialog yet; OpenFolder already does exactly this, so
                // reuse it rather than duplicate it.
                let _ = request_tx.send(PortalRequest::OpenFolder { title, response_tx });
            } else {
                let _ = request_tx.send(PortalRequest::OpenFile { title, response_tx });
            }
        })
    }

    #[zbus(name = "SaveFile")]
    fn save_file(
        &self,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let default_name = options
            .get("current_name")
            .and_then(|v| v.clone().downcast::<String>().ok())
            .unwrap_or_else(|| "Untitled".to_string());

        let title = title.to_string();
        let request_tx = self.request_tx.clone();

        self.begin_request(move |response_tx| {
            let _ = request_tx.send(PortalRequest::SaveFile {
                title,
                default_name,
                response_tx,
            });
        })
    }

    #[zbus(name = "SaveFiles")]
    fn save_files(
        &self,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let current_folder = option_path(&options, "current_folder");
        let files = option_path_list(&options, "files");
        let files = if files.is_empty() {
            vec!["Untitled".to_string()]
        } else {
            files
        };

        let title = title.to_string();
        let request_tx = self.request_tx.clone();

        // The actual folder-picking + folder/filename joining happens on
        // the GTK thread (see the `PortalRequest::SaveFiles` arm in
        // main.rs) so it can send back a plain `Vec<String>` of already-
        // joined target paths -- `begin_request`'s response handling
        // below then turns each of those into a `uris` entry exactly the
        // same way it already does for OpenFile/SaveFile/OpenFolder,
        // with no changes needed there.
        self.begin_request(move |response_tx| {
            let _ = request_tx.send(PortalRequest::SaveFiles {
                title,
                current_folder,
                files,
                response_tx,
            });
        })
    }
}

impl FileChooserPortal {
    /// Shared plumbing for every method above: allocate a fresh request
    /// object path, register a `Request` object there (so `Close()` works
    /// immediately), hand the caller a closure to actually kick off the
    /// GTK-side dialog (via the existing `PortalRequest` channel), and
    /// spawn the thread that waits for that dialog's result and emits the
    /// `Response` signal once it's in. Returns the request path -- the
    /// method itself returns right away, same as the real spec expects.
    fn begin_request(
        &self,
        start_dialog: impl FnOnce(mpsc::Sender<PortalResponse>) + Send + 'static,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let token = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path_string = format!("/org/freedesktop/portal/desktop/request/mitosfiles/r{token}");
        let owned_path = OwnedObjectPath::try_from(path_string)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Bad request path: {e}")))?;

        let responded = Arc::new(AtomicBool::new(false));

        let request_obj = RequestObject {
            responded: responded.clone(),
        };

        self.connection
            .object_server()
            .at(owned_path.clone(), request_obj)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Couldn't create request: {e}")))?;

        let (response_tx, response_rx) = mpsc::channel();
        start_dialog(response_tx);

        let connection = self.connection.clone();
        let emit_path = owned_path.clone();

        std::thread::spawn(move || {
            // The GTK thread only ever sends one reply per request, so
            // this either gets it or the sender was dropped (window
            // closed some other way) -- either way we're done waiting.
            let outcome = response_rx.recv();

            // `Close()` may have already answered this request (user
            // cancelled through some other path) -- don't double-reply.
            if responded.swap(true, Ordering::SeqCst) {
                return;
            }

            let (response_code, results) = match outcome {
                Ok(PortalResponse::Selected(paths)) if !paths.is_empty() => {
                    let uris: Vec<Value> = paths
                        .iter()
                        .map(|p| Value::from(path_to_file_uri(p)))
                        .collect();
                    let mut results: HashMap<String, Value> = HashMap::new();
                    results.insert("uris".to_string(), Value::from(uris));
                    (0u32, results)
                }
                Ok(PortalResponse::Selected(_)) | Ok(PortalResponse::Cancelled) => {
                    (1u32, HashMap::new())
                }
                Ok(PortalResponse::Error(_)) | Err(_) => (2u32, HashMap::new()),
            };

            let _ = connection.emit_signal(
                Option::<&str>::None,
                emit_path.clone(),
                "org.freedesktop.portal.Request",
                "Response",
                &(response_code, results),
            );

            let _ = connection
                .object_server()
                .remove::<RequestObject, _>(emit_path);
        });

        Ok(owned_path)
    }
}

struct RequestObject {
    responded: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.portal.Request")]
impl RequestObject {
    fn close(&self) {
        // Just marks the request answered so the waiting thread's later
        // reply becomes a no-op; MITOS Files doesn't currently have a way
        // to force-close an already-open GTK dialog from here. The
        // dialog stays open until the user acts on it, but its result
        // will be silently dropped instead of (wrongly) reported.
        self.responded.store(true, Ordering::SeqCst);
    }
}

fn option_bool(options: &HashMap<String, OwnedValue>, key: &str) -> bool {
    let Some(value) = options.get(key) else {
        return false;
    };

    value.clone().downcast::<bool>().unwrap_or(false)
}

/// Reads a `ay` (byte-array) option as a path/string -- the D-Bus
/// convention portals use for filesystem paths, since paths aren't
/// guaranteed valid UTF-8. `None` on a missing key or a value that isn't
/// actually a byte array, same permissive-fallback shape as `option_bool`.
fn option_path(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = options.get(key)?;
    let bytes = value.clone().downcast::<Vec<u8>>().ok()?;
    let bytes = bytes.strip_suffix(&[0u8]).unwrap_or(&bytes);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Same idea as `option_path`, for the `aay` list of suggested filenames
/// `SaveFiles` takes. Empty (rather than `None`) on a missing/wrong-typed
/// key, since the caller already has a sensible fallback for "no names".
fn option_path_list(options: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    let Some(value) = options.get(key) else {
        return Vec::new();
    };

    let Ok(list) = value.clone().downcast::<Vec<Vec<u8>>>() else {
        return Vec::new();
    };

    list.into_iter()
        .map(|bytes| {
            let bytes = bytes.strip_suffix(&[0u8]).unwrap_or(&bytes);
            String::from_utf8_lossy(bytes).into_owned()
        })
        .collect()
}

fn path_to_file_uri(path: &str) -> String {
    gtk::gio::File::for_path(path).uri().to_string()
}
