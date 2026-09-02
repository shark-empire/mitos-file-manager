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
