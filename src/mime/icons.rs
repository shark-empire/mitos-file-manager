use gtk::gio;
use gtk::prelude::*;

pub fn icon_name_for_mime(mime: &str, is_dir: bool) -> String {
    if is_dir || mime == "inode/directory" {
        return "folder".to_string();
    }

    if let Some(icon) = gio::content_type_get_icon(mime) {
        if let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() {
            if let Some(name) = themed.names().first() {
                return name.to_string();
            }
        }
    }

    // Fallbacks for environments where MIME icons are incomplete.
    if mime.starts_with("image/") {
        return "image-x-generic".to_string();
    }

    if mime.starts_with("video/") {
        return "video-x-generic".to_string();
    }

    if mime.starts_with("audio/") {
        return "audio-x-generic".to_string();
    }

    if mime.starts_with("text/") {
        return "text-x-generic".to_string();
    }

    if mime == "application/pdf" {
        return "x-office-document".to_string();
    }

    if mime.contains("archive")
        || mime.contains("compressed")
        || mime.contains("zip")
        || mime.contains("tar")
    {
        return "package-x-generic".to_string();
    }

    if mime.contains("executable") || mime.contains("shellscript") {
        return "application-x-executable".to_string();
    }

    "text-x-generic".to_string()
}
