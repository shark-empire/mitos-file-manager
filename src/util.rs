use gtk::glib;
use gtk::glib::object::ObjectExt;

/// Store typed Rust-side data on a GObject's qdata.
///
/// This is the one place `ObjectExt::set_data` (an `unsafe fn` — it doesn't
/// track the type it was called with) is invoked; every call site in this
/// crate goes through here so the `unsafe` surface stays in one spot.
pub fn set_obj_data<O: glib::object::ObjectType, T: 'static>(obj: &O, key: &str, value: T) {
    unsafe {
        obj.set_data(key, value);
    }
}

/// Fetch a clone of typed Rust-side data previously stored with
/// [`set_obj_data`]. Returns `None` if nothing was stored under `key`.
///
/// `ObjectExt::data` returns a raw `NonNull<T>` (it's `unsafe fn` for the
/// same reason as `set_data`); this dereferences and clones it into an
/// owned `T` so every call site gets a normal, safe value back.
pub fn get_obj_data<O: glib::object::ObjectType, T: Clone + 'static>(
    obj: &O,
    key: &str,
) -> Option<T> {
    unsafe { obj.data::<T>(key).map(|ptr| ptr.as_ref().clone()) }
}

/// Remove and return typed Rust-side data previously stored with
/// [`set_obj_data`], taking ownership directly instead of cloning it.
/// Returns `None` if nothing was stored under `key`. Needed for types
/// that don't implement `Clone` -- e.g. `glib::SignalHandlerId`, which
/// deliberately isn't `Clone` (to prevent accidentally disconnecting the
/// same signal handler twice), so [`get_obj_data`] can't be used for it
/// at all.
///
/// `ObjectExt::steal_data` is `unsafe fn` for the same reason as
/// `data`/`set_data` above; this keeps it behind the same safe wrapper.
pub fn take_obj_data<O: glib::object::ObjectType, T: 'static>(obj: &O, key: &str) -> Option<T> {
    unsafe { obj.steal_data::<T>(key) }
}

/// Decode `%XX` escapes, as found in `file://` URIs and `.trashinfo` files.
/// A malformed or truncated escape is left exactly as written, and any bytes
/// that don't form valid UTF-8 are replaced rather than rejected.
///
/// (Works on bytes throughout: slicing the string at `i + 1..i + 3` would
/// panic when a `%` is followed by a multi-byte character.)
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }

        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

#[cfg(test)]
mod percent_tests {
    use super::percent_decode;

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%C3%A9t%C3%A9"), "\u{e9}t\u{e9}");
        assert_eq!(
            percent_decode("/home/u/My%20Files/100%25"),
            "/home/u/My Files/100%"
        );
    }

    #[test]
    fn a_trailing_escape_is_still_decoded() {
        assert_eq!(percent_decode("%41"), "A");
    }

    #[test]
    fn bad_or_truncated_escapes_are_left_alone() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("50%4"), "50%4");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn a_percent_before_a_multibyte_character_does_not_panic() {
        assert_eq!(percent_decode("%\u{e9}x"), "%\u{e9}x");
        assert_eq!(percent_decode("a%\u{20ac}"), "a%\u{20ac}");
    }
}

#[cfg(test)]
pub mod test_support {
    use std::path::PathBuf;

    /// A fresh, empty directory unique to `tag` (and to this test process),
    /// for tests that need a real filesystem. Give every test its own tag:
    /// tests run in parallel.
    pub fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mitos-fm-test-{}-{tag}", std::process::id()));

        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");

        dir
    }
}
