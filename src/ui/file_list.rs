use crate::ui::item_object::ItemObject;
use gtk::prelude::*;

/// One compact row describing a file -- icon, permissions, name, size -- for
/// places that list several items as text rather than as a grid of icons
/// (the multi-item Properties dialog).
///
/// Everything shown comes straight off the `ItemObject`, which already
/// carries the formatted permission string and size populated from the
/// filesystem metadata module.
pub fn render_file_row(file: &ItemObject) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    row.set_margin_top(2);
    row.set_margin_bottom(2);
    row.set_margin_start(6);
    row.set_margin_end(6);

    let icon = gtk::Image::from_icon_name(&file.icon_name());

    // Access the pre-formatted permissions directly from the object properties
    let perms = gtk::Label::new(Some(&file.permissions()));
    perms.add_css_class("monospace");

    let name = gtk::Label::new(Some(&file.name()));
    name.set_halign(gtk::Align::Start);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let size = gtk::Label::new(Some(&file.size_str()));
    size.add_css_class("dim-label");

    row.append(&icon);
    row.append(&perms);
    row.append(&name);
    row.append(&size);

    row.upcast()
}
