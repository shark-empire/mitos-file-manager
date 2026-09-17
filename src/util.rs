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
