use gtk::gio;
use gtk::prelude::*;

use crate::filesystem::directory::Item;
use crate::ui::item_object::ItemObject;

pub fn create_model() -> (gio::ListStore, gtk::MultiSelection) {
    let store = gio::ListStore::new::<ItemObject>();
    let selection = gtk::MultiSelection::new(Some(store.clone()));

    (store, selection)
}

pub fn create_grid_view(selection: &gtk::MultiSelection) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(move |_, item, _| {
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
        picture.set_keep_aspect_ratio(true);
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

        item.set_data("stack", stack);
        item.set_data("icon", icon);
        item.set_data("picture", picture);
        item.set_data("label", label);
    });

    factory.connect_bind(move |_, item| {
        let item_obj = item.item().and_downcast::<ItemObject>().unwrap();

        let stack = item.data::<gtk::Stack>("stack").unwrap();
        let icon = item.data::<gtk::Image>("icon").unwrap();
        let picture = item.data::<gtk::Picture>("picture").unwrap();
        let label = item.data::<gtk::Label>("label").unwrap();

        icon.set_icon_name(Some(&item_obj.icon_name()));
        label.set_label(&item_obj.name());

        let thumbnail_path = item_obj.thumbnail_path();

        if thumbnail_path.is_empty() {
            picture.set_filename(None::<&str>);
            stack.set_visible_child(&icon);
        } else {
            picture.set_filename(Some(thumbnail_path.as_str()));
            stack.set_visible_child(&picture);
        }
    });

    let grid_view = gtk::GridView::new(Some(selection), Some(factory));

    grid_view.set_max_columns(20);
    grid_view.set_min_columns(3);
    grid_view.set_enable_rubberband(true);

    grid_view
}

pub fn render(store: &gio::ListStore, items: &[Item]) {
    store.remove_all();

    for item in items {
        store.append(&ItemObject::new(item));
    }
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
