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

    content_type.to_string()
}

/// The MIME type of `path` judged by its *name* -- no file access at all
/// when the extension is conclusive (".txt", ".png", ".rs", ...). Only a
/// name that settles nothing (no extension, or one nobody has heard of)
/// falls back to `guess_mime`, which looks inside the file.
///
/// Listing a folder used to sniff every single file, which is what made big
/// folders slow to open; most files carry a perfectly good extension.
pub fn guess_mime_by_name(path: &Path, name: &str) -> String {
    let (content_type, uncertain) = gio::content_type_guess(Some(name), &[]);

    if uncertain {
        guess_mime(path)
    } else {
        content_type.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conclusive_extension_is_answered_without_touching_the_file() {
        // These paths don't exist: if the name alone weren't enough, the
        // sniffing fallback would come back as octet-stream.
        assert!(
            guess_mime_by_name(Path::new("/no/such/photo.png"), "photo.png").starts_with("image/")
        );
        assert!(
            guess_mime_by_name(Path::new("/no/such/notes.txt"), "notes.txt").starts_with("text/")
        );
    }

    #[test]
    fn an_unknown_name_still_gets_an_answer() {
        assert!(!guess_mime_by_name(Path::new("/no/such/mystery"), "mystery").is_empty());
    }
}
