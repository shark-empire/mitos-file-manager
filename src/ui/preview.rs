use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Image, Label, Orientation, Picture, ScrolledWindow, Separator, Stack, TextView,
};
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::mime::thumbnail;
use crate::operations::archive;
use crate::util::{get_obj_data, set_obj_data};

/// Longest side, in pixels, of the picture shown in the preview panel.
/// Images are decoded straight to this size rather than at full resolution,
/// which for a large photo is the difference between a megabyte and a
/// hundred.
const PREVIEW_IMAGE_SIZE: i32 = 512;

/// How many names of a folder's contents / an archive's entries the preview
/// lists before saying "...and more".
const PREVIEW_LIST_LIMIT: usize = 60;

/// Stop counting a folder's items here -- "5000+ items" is as informative as
/// the exact figure, without reading a huge directory just to hover over it.
const FOLDER_COUNT_CAP: usize = 5000;

#[derive(Clone)]
struct PreviewWidgets {
    stack: Stack,
    image: Picture,
    text_view: TextView,
    icon: Image,
    name_label: Label,
    type_label: Label,
    size_label: Label,
    modified_label: Label,
    path_label: Label,
    permissions_label: Label,
}

pub fn build() -> (ScrolledWindow, GtkBox) {
    let container = GtkBox::new(Orientation::Vertical, 8);

    container.set_margin_top(8);
    container.set_margin_bottom(8);
    container.set_margin_start(8);
    container.set_margin_end(8);

    // Preview area: image / text / icon
    let stack = Stack::new();
    stack.set_height_request(200);

    let image = Picture::new();
    image.set_can_shrink(true);
    image.set_content_fit(gtk::ContentFit::Contain);
    image.set_height_request(200);
    stack.add_named(&image, Some("image"));

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);

    let text_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    text_scroll.set_child(Some(&text_view));
    stack.add_named(&text_scroll, Some("text"));

    let icon = Image::new();
    icon.set_pixel_size(96);
    icon.set_valign(gtk::Align::Center);
    stack.add_named(&icon, Some("icon"));

    container.append(&stack);

    let sep = Separator::new(Orientation::Horizontal);
    container.append(&sep);

    // Metadata labels
    let name_label = Label::new(None);
    name_label.set_wrap(true);
    name_label.set_halign(gtk::Align::Start);
    name_label.add_css_class("heading");
    container.append(&name_label);

    let type_label = Label::new(None);
    type_label.set_halign(gtk::Align::Start);
    container.append(&type_label);

    let size_label = Label::new(None);
    size_label.set_halign(gtk::Align::Start);
    container.append(&size_label);

    let modified_label = Label::new(None);
    modified_label.set_halign(gtk::Align::Start);
    container.append(&modified_label);

    let path_label = Label::new(None);
    path_label.set_wrap(true);
    path_label.set_halign(gtk::Align::Start);
    container.append(&path_label);

    let permissions_label = Label::new(None);
    permissions_label.set_halign(gtk::Align::Start);
    container.append(&permissions_label);

    let widgets = PreviewWidgets {
        stack,
        image,
        text_view,
        icon,
        name_label,
        type_label,
        size_label,
        modified_label,
        path_label,
        permissions_label,
    };

    set_obj_data(&container, "preview-widgets", widgets);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&container));
    scrolled.set_width_request(280);
    scrolled.set_visible(false);

    (scrolled, container)
}

pub fn update(container: &GtkBox, item: Option<&crate::ui::item_object::ItemObject>) {
    let Some(widgets) = get_obj_data::<_, PreviewWidgets>(container, "preview-widgets") else {
        return;
    };

    let Some(item) = item else {
        widgets.name_label.set_label("No selection");
        widgets.type_label.set_label("");
        widgets.size_label.set_label("");
        widgets.modified_label.set_label("");
        widgets.path_label.set_label("");
        widgets.permissions_label.set_label("");
        widgets.stack.set_visible_child_name("icon");
        widgets.icon.set_icon_name(Some("edit-find-symbolic"));
        return;
    };

    let path = item.get_path();
    let mime = item.mime_type();

    widgets.name_label.set_label(&item.name());
    widgets.type_label.set_label(&format!("Type: {}", mime));
    widgets
        .size_label
        .set_label(&format!("Size: {}", item.size_str()));
    widgets
        .modified_label
        .set_label(&format!("Modified: {}", item.modified_str()));
    widgets
        .path_label
        .set_label(&format!("Path: {}", path.display()));
    widgets
        .permissions_label
        .set_label(&format!("Permissions: {}", item.permissions()));

    // Which page of the preview stack to show, and what goes on it.
    let text = if item.is_dir() {
        Some(describe_folder(&path))
    } else if mime.starts_with("image/") {
        if show_image(&widgets, item, &path) {
            return;
        }

        None
    } else if mime.starts_with("video/") {
        // A frame grabbed for the icon view is reused if it's already been
        // made; the preview never runs ffmpeg itself.
        let cached = thumbnail::thumbnail_path_for(&path);

        if !cached.is_empty() {
            widgets.image.set_filename(Some(&cached));
            widgets.stack.set_visible_child_name("image");
            return;
        }

        None
    } else if archive::is_supported_archive(&path) {
        describe_archive(&path)
    } else if is_text_mime(&mime) {
        read_text_preview(&path)
    } else {
        None
    };

    match text {
        Some(content) => {
            widgets.text_view.buffer().set_text(&content);
            widgets.stack.set_visible_child_name("text");
        }
        None => {
            widgets.stack.set_visible_child_name("icon");
            widgets.icon.set_icon_name(Some(&item.icon_name()));
        }
    }
}

/// Show an image scaled down to `PREVIEW_IMAGE_SIZE`. Returns false (so the
/// caller falls back to the file-type icon) for files over the "max
/// thumbnail size" setting or that no installed loader can read.
fn show_image(widgets: &PreviewWidgets, item: &crate::ui::item_object::ItemObject, path: &Path) -> bool {
    let size = item.size();

    if size == 0 || size > crate::config::settings::thumbnail_max_bytes() {
        return false;
    }

    let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(
        path,
        PREVIEW_IMAGE_SIZE,
        PREVIEW_IMAGE_SIZE,
        true,
    ) else {
        return false;
    };

    // Phone photos are stored sideways with an orientation flag.
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);

    widgets
        .image
        .set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&pixbuf)));
    widgets.stack.set_visible_child_name("image");

    true
}

/// "12 items" and the first few names, for a folder.
fn describe_folder(path: &Path) -> String {
    let Ok(entries) = fs::read_dir(path) else {
        return "This folder can't be read.".to_string();
    };

    let mut names: Vec<String> = Vec::new();
    let mut count = 0usize;

    for entry in entries.flatten() {
        count += 1;

        if names.len() < PREVIEW_LIST_LIMIT {
            names.push(entry.file_name().to_string_lossy().to_string());
        }

        if count >= FOLDER_COUNT_CAP {
            break;
        }
    }

    names.sort_by_key(|name| name.to_lowercase());

    let header = match count {
        0 => "Empty folder".to_string(),
        1 => "1 item".to_string(),
        n if n >= FOLDER_COUNT_CAP => format!("{n}+ items"),
        n => format!("{n} items"),
    };

    if names.is_empty() {
        return header;
    }

    let mut text = format!("{header}\n\n{}", names.join("\n"));

    if count > names.len() {
        text.push_str("\n\u{2026}");
    }

    text
}

/// The entries inside an archive, without extracting it.
fn describe_archive(path: &Path) -> Option<String> {
    let listing = archive::list_entries(path, PREVIEW_LIST_LIMIT).ok()?;

    let mut text = format!("{} entries shown\n\n{}", listing.entries.len(), listing.entries.join("\n"));

    if listing.truncated {
        text.push_str("\n\u{2026} and more");
    }

    Some(text)
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("javascript")
        || mime.contains("shellscript")
        || mime.contains("python")
        || mime.contains("rust")
        || mime.contains("html")
        || mime.contains("css")
        || mime.contains("yaml")
        || mime.contains("toml")
}

fn read_text_preview(path: &std::path::Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;

    // Only preview files under 1 MB
    if metadata.len() > 1_000_000 {
        return None;
    }

    let mut file = fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; 4096];
    let bytes_read = file.read(&mut buffer).ok()?;
    buffer.truncate(bytes_read);

    // A NUL byte means it isn't really text (whatever its extension claims).
    if buffer.contains(&0) {
        return None;
    }

    // Lossy: the 4 KB cut can land in the middle of a multi-byte character,
    // and a strict conversion would then reject a perfectly good text file.
    let mut text = String::from_utf8_lossy(&buffer).into_owned();

    if metadata.len() > bytes_read as u64 {
        text.push_str("\n\u{2026}");
    }

    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn a_folder_is_described_by_its_item_count_and_names() {
        let dir = scratch_dir("preview-folder");
        fs::write(dir.join("b.txt"), "b").unwrap();
        fs::write(dir.join("A.txt"), "a").unwrap();

        assert_eq!(describe_folder(&dir), "2 items\n\nA.txt\nb.txt");

        let empty = dir.join("empty");
        fs::create_dir(&empty).unwrap();
        assert_eq!(describe_folder(&empty), "Empty folder");

        assert_eq!(describe_folder(&dir.join("nope")), "This folder can't be read.");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_preview_survives_a_multibyte_character_at_the_cut() {
        let dir = scratch_dir("preview-text");
        let file = dir.join("long.txt");

        // 4095 ASCII bytes, then a two-byte character straddling the 4 KB cut.
        let mut content = "a".repeat(4095);
        content.push('\u{e9}');
        content.push_str(&"b".repeat(100));
        fs::write(&file, &content).unwrap();

        let preview = read_text_preview(&file).expect("still previewable");

        assert!(preview.starts_with("aaaa"));
        assert!(preview.ends_with('\u{2026}'));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_files_and_huge_files_are_not_previewed_as_text() {
        let dir = scratch_dir("preview-binary");

        let binary = dir.join("blob.txt");
        fs::write(&binary, b"text then \0 binary").unwrap();
        assert!(read_text_preview(&binary).is_none());

        let small = dir.join("small.txt");
        fs::write(&small, "hello").unwrap();
        assert_eq!(read_text_preview(&small).as_deref(), Some("hello"));

        let _ = fs::remove_dir_all(&dir);
    }
}
