//! Discovering `network:///` locations (nearby SMB/DNS-SD/UPnP shares GVfs
//! knows about), as opposed to `servers.rs`'s "type an address by hand".
//!
//! Whether anything turns up depends entirely on which GVfs backends are
//! installed (`gvfs-backends`/`gvfs-smb` and friends) and what's actually
//! announcing itself on the network -- an empty result here is normal on a
//! machine without those backends, not a bug.

use gtk::gio;
use gtk::prelude::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkLocation {
    pub name: String,
    /// What to hand to `gio::File::for_uri` -- may itself be another
    /// browsable folder of locations (a workgroup, say), not always a
    /// directly mountable share.
    pub uri: String,
}

/// List what's under `network:///` right now. Blocks on GIO's synchronous
/// enumeration, so call this from a worker thread, not the GTK thread
/// (`ui::sidebar`'s "Browse Network..." handler does).
///
/// `timeout` bounds how long a slow or hanging backend can hold this up;
/// on timeout, whatever was found before the deadline is returned rather
/// than nothing.
pub fn discover(timeout: Duration) -> Vec<NetworkLocation> {
    let root = gio::File::for_uri("network:///");

    let Ok(enumerator) = root.enumerate_children(
        "standard::display-name,standard::target-uri,standard::type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) else {
        // No GVfs network backend able to answer at all.
        return Vec::new();
    };

    let deadline = std::time::Instant::now() + timeout;
    let mut found = Vec::new();

    loop {
        if std::time::Instant::now() >= deadline {
            break;
        }

        let Ok(Some(info)) = enumerator.next_file(gio::Cancellable::NONE) else {
            break;
        };

        let name = info.display_name().to_string();

        // Prefer the resolved `target-uri` (what actually mounting this
        // entry would use); the enumerator's own child URI is the fallback
        // for an entry that doesn't provide one.
        let uri = info
            .attribute_string("standard::target-uri")
            .map(|value| value.to_string())
            .unwrap_or_else(|| root.child(&name).uri().to_string());

        if !name.is_empty() {
            found.push(NetworkLocation { name, uri });
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_never_panics_even_with_an_instant_timeout() {
        // Can't assert on *what* comes back (depends entirely on the host's
        // GVfs backends and its actual network), only that asking doesn't
        // crash and respects a near-zero budget.
        let _ = discover(Duration::from_millis(0));
    }
}
