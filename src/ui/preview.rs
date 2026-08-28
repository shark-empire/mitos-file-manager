use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Image, Label, Orientation, Picture, ScrolledWindow, Separator, Stack,
    TextView,
};
use std::fs;
use std::io::Read;

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
    image.set_keep_aspect_ratio(true);
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

    container.set_data("preview-widgets", widgets);

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
    let Some(widgets) = container.data::<PreviewWidgets>("preview-widgets") else {
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
    widgets.size_label.set_label(&format!("Size: {}", item.size_str()));
    widgets.modified_label.set_label(&format!("Modified: {}", item.modified_str()));
    widgets.path_label.set_label(&format!("Path: {}", path.display()));
    widgets.permissions_label.set_label(&format!("Permissions: {}", item.permissions()));

    if mime.starts_with("image/") {
        widgets.image.set_filename(Some(&path));
        widgets.stack.set_visible_child_name("image");
    } else if is_text_mime(&mime) {
        if let Some(content) = read_text_preview(&path) {
            widgets.text_view.buffer().set_text(&content);
            widgets.stack.set_visible_child_name("text");
        } else {
            widgets.stack.set_visible_child_name("icon");
            widgets.icon.set_icon_name(Some(&item.icon_name()));
        }
    } else {
        widgets.stack.set_visible_child_name("icon");
        widgets.icon.set_icon_name(Some(&item.icon_name()));
    }
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

    String::from_utf8(buffer).ok()
}
