use gtk::gio;
use gtk::prelude::*;
use std::path::Path;

pub fn guess_mime(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_string();
    }

    let file = gio::File::for_path(path);

    if let Ok(info) = file.query_info(
        "standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        if let Some(content_type) = info.content_type() {
            return content_type.to_string();
        }
    }

    let (content_type, _uncertain) = gio::content_type_guess(path.to_str(), &[]);

    content_type
}
