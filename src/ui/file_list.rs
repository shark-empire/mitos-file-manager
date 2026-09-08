use gtk::prelude::*;
use crate::ui::item_object::ItemObject;

// Use ItemObject which already holds the formatted permission string 
// populated from your filesystem metadata module.
pub fn render_file_row(file: &ItemObject) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    // Access the pre-formatted permissions directly from the object properties
    let perms = file.permissions(); 

    row.append(&gtk::Label::new(Some(&perms)));
    row.append(&gtk::Label::new(Some(&file.name())));

    row.upcast()
}
