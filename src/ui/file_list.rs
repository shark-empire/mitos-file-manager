use gtk::prelude::*;
use mitos_utils::common::permissions::format_permissions;

fn render_file_row(file: &FileEntry) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    // Reuse the exact formatting logic from `mitos-ls`
    let perms = format_permissions(file.mode);

    row.append(&gtk::Label::new(Some(&perms)));
    row.append(&gtk::Label::new(Some(&file.name)));

    row.upcast()
}
