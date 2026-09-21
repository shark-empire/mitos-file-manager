use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::filesystem::directory::Item;
use crate::mime::thumbnail;
use crate::ui::item_object::ItemObject;
use crate::util::{get_obj_data, set_obj_data, take_obj_data};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub fn create_model() -> (gio::ListStore, gtk::MultiSelection) {
    let store = gio::ListStore::new::<ItemObject>();
    let selection = gtk::MultiSelection::new(Some(store.clone()));

    (store, selection)
}

fn apply_thumbnail(stack: &gtk::Stack, icon: &gtk::Image, picture: &gtk::Picture, path: &str) {
    if path.is_empty() {
        picture.set_filename(None::<&str>);
        stack.set_visible_child(icon);
    } else {
        picture.set_filename(Some(path));
        stack.set_visible_child(picture);
    }
}

pub fn create_grid_view(selection: &gtk::MultiSelection) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("Needs to be ListItem");

        let container = gtk::Box::new(gtk::Orientation::Vertical, 4);

        container.set_width_request(110);
        container.set_height_request(130);

        container.set_margin_top(4);
        container.set_margin_bottom(4);
        container.set_margin_start(4);
        container.set_margin_end(4);

        let stack = gtk::Stack::new();
        stack.set_halign(gtk::Align::Center);

        let icon = gtk::Image::new();
        icon.set_pixel_size(48);
        icon.set_halign(gtk::Align::Center);

        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_width_request(72);
        picture.set_height_request(72);
        picture.set_halign(gtk::Align::Center);

        stack.add_child(&icon);
        stack.add_child(&picture);
        stack.set_visible_child(&icon);

        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_halign(gtk::Align::Center);
        label.set_max_width_chars(12);

        container.append(&stack);
        container.append(&label);

        item.set_child(Some(&container));

        set_obj_data(item, "stack", stack);
        set_obj_data(item, "icon", icon);
        set_obj_data(item, "picture", picture);
        set_obj_data(item, "label", label);
    });

    factory.connect_bind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("Needs to be ListItem");

        let Some(item_obj) = item.item().and_downcast::<ItemObject>() else {
            return;
        };

        let stack: Option<gtk::Stack> = get_obj_data(item, "stack");
        let icon: Option<gtk::Image> = get_obj_data(item, "icon");
        let picture: Option<gtk::Picture> = get_obj_data(item, "picture");
        let label: Option<gtk::Label> = get_obj_data(item, "label");

        let (Some(stack), Some(icon), Some(picture), Some(label)) = (stack, icon, picture, label)
        else {
            return;
        };

        icon.set_icon_name(Some(&item_obj.icon_name()));
        label.set_label(&item_obj.name());

        apply_thumbnail(&stack, &icon, &picture, &item_obj.thumbnail_path());

        // Video thumbnails aren't ready at bind time (see below), so stay
        // in sync if `thumbnail-path` changes later. `GridView` recycles
        // `ListItem`s as you scroll, so this has to be disconnected in
        // `connect_unbind` below -- otherwise a stale handler could later
        // fire and repaint whatever row this item got recycled into.
        let stack_for_notify = stack.clone();
        let icon_for_notify = icon.clone();
        let picture_for_notify = picture.clone();

        let handler_id = item_obj.connect_notify_local(Some("thumbnail-path"), move |obj, _| {
            apply_thumbnail(
                &stack_for_notify,
                &icon_for_notify,
                &picture_for_notify,
                &obj.thumbnail_path(),
            );
        });

        set_obj_data(item, "thumbnail-signal-handler", handler_id);

        // Thumbnails are worked out here, lazily, for the rows that are
        // actually on screen -- not for every file when a big folder is
        // listed. A cached one is picked up at once; otherwise it's made on
        // a background worker and the `notify` handler above swaps it in
        // when it arrives.
        request_thumbnail_if_needed(item, &item_obj);
    });

    factory.connect_unbind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("Needs to be ListItem");

        let Some(item_obj) = item.item().and_downcast::<ItemObject>() else {
            return;
        };

        if let Some(handler_id) =
            take_obj_data::<_, glib::SignalHandlerId>(item, "thumbnail-signal-handler")
        {
            item_obj.disconnect(handler_id);
        }

        // This row is about to show something else: tell the worker not to
        // bother finishing a thumbnail nobody is looking at any more.
        if let Some(cancel) = take_obj_data::<_, Arc<AtomicBool>>(item, "thumbnail-cancel") {
            cancel.store(true, Ordering::Relaxed);
        }
    });

    let grid_view = gtk::GridView::new(Some(selection.clone()), Some(factory));

    grid_view.set_max_columns(20);
    grid_view.set_min_columns(3);
    grid_view.set_enable_rubberband(true);

    grid_view
}

/// Look up -- or ask a background worker to make -- the thumbnail for the
/// file in `item`'s row, if it wants one. Cheap when there's nothing to do.
fn request_thumbnail_if_needed(item: &gtk::ListItem, item_obj: &ItemObject) {
    if !item_obj.thumbnail_path().is_empty() || item_obj.is_dir() {
        return;
    }

    // A file nothing could make a thumbnail for isn't asked again every
    // time it scrolls back into view.
    if get_obj_data::<_, bool>(item_obj, "thumbnail-failed").unwrap_or(false) {
        return;
    }

    let mime = item_obj.mime_type();

    if !thumbnail::wants_thumbnail(&mime, item_obj.size()) {
        return;
    }

    let path = item_obj.get_path();

    let cached = thumbnail::thumbnail_path_for(&path);

    if !cached.is_empty() {
        item_obj.set_thumbnail_path(cached);
        return;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    set_obj_data(item, "thumbnail-cancel", cancel.clone());

    let receiver = thumbnail::request_thumbnail(path, mime, cancel.clone());

    // Weak, so a row that scrolls away before the worker finishes doesn't
    // keep its `ItemObject` alive.
    let item_obj_weak = item_obj.downgrade();

    glib::MainContext::default().spawn_local(async move {
        let Ok(result) = receiver.recv().await else {
            return;
        };

        // The row scrolled away first: the worker gave up on purpose, which
        // says nothing about whether the file can be thumbnailed.
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        let Some(item_obj) = item_obj_weak.upgrade() else {
            return;
        };

        match result {
            Some(cache_path) => item_obj.set_thumbnail_path(cache_path),
            None => set_obj_data(&item_obj, "thumbnail-failed", true),
        }
    });
}

/// Replace the store's contents with `items` in one go: a single
/// `items-changed` signal (and one relayout) instead of one per item.
pub fn render(store: &gio::ListStore, items: &[Item]) {
    let objects: Vec<ItemObject> = items.iter().map(ItemObject::new).collect();

    store.splice(0, store.n_items(), &objects);
}

/// How many rows are shown immediately, and how many are added per step
/// after that.
const FIRST_CHUNK: usize = 300;
const NEXT_CHUNK: usize = 500;

/// Like `render`, but for a listing that may be huge: the first screenful
/// goes in at once so the folder appears immediately, and the rest follows
/// in chunks from the main loop -- the window stays responsive while a
/// 100,000-file folder fills in, instead of freezing until every row
/// object exists. `still_current` is asked before each chunk; once it says
/// no (the tab moved on, or a newer listing replaced this one) the
/// remainder is dropped.
pub fn render_progressive(
    store: &gio::ListStore,
    items: Vec<Item>,
    still_current: impl Fn() -> bool + 'static,
) {
    let first_end = items.len().min(FIRST_CHUNK);
    let first: Vec<ItemObject> = items[..first_end].iter().map(ItemObject::new).collect();

    store.splice(0, store.n_items(), &first);

    if first_end >= items.len() {
        return;
    }

    let store = store.clone();
    let mut cursor = first_end;

    glib::timeout_add_local(Duration::from_millis(2), move || {
        if !still_current() {
            return glib::ControlFlow::Break;
        }

        let end = (cursor + NEXT_CHUNK).min(items.len());
        let chunk: Vec<ItemObject> = items[cursor..end].iter().map(ItemObject::new).collect();

        store.splice(store.n_items(), 0, &chunk);
        cursor = end;

        if cursor >= items.len() {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

pub fn selected_items(selection: &gtk::MultiSelection, store: &gio::ListStore) -> Vec<ItemObject> {
    let mut selected = Vec::new();

    for i in 0..store.n_items() {
        if selection.is_selected(i) {
            if let Some(obj) = store.item(i) {
                if let Some(item_obj) = obj.downcast_ref::<ItemObject>() {
                    selected.push(item_obj.clone());
                }
            }
        }
    }

    selected
}
