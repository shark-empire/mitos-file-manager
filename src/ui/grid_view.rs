// src/ui/grid_view.rs
use gtk::prelude::*;
use gtk::gio;
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
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        box_.set_width_request(100);
        box_.set_height_request(110);
        box_.set_margin_top(4);
        box_.set_margin_bottom(4);
        box_.set_margin_start(4);
        box_.set_margin_end(4);

        let icon = gtk::Image::new();
        icon.set_pixel_size(48);
        icon.set_halign(gtk::Align::Center);

        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_halign(gtk::Align::Center);
        label.set_max_width_chars(12);

        box_.append(&icon);
        box_.append(&label);

        item.set_child(Some(&box_));
        item.set_data("icon", icon);
        item.set_data("label", label);
    });

    factory.connect_bind(move |_, item| {
        let item_obj = item.item().and_downcast::<ItemObject>().unwrap();
        let icon = item.data::<gtk::Image>("icon").unwrap();
        let label = item.data::<gtk::Label>("label").unwrap();

        icon.set_icon_name(Some(&item_obj.icon_name()));
        label.set_label(&item_obj.name());
    });

    let grid_view = gtk::GridView::new(Some(selection), Some(factory));
    grid_view.set_max_columns(20);
    grid_view.set_min_columns(3);
    grid_view.set_enable_rubberband(true); // Allows drag-to-select
    
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
