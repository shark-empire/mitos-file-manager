use gtk::prelude::*;
use gtk::{ApplicationWindow, Dialog, Entry, Label, ResponseType};

pub fn show_text_dialog<F>(
    parent: &ApplicationWindow,
    title: &str,
    initial: &str,
    ok_label: &str,
    on_accept: F,
) where
    F: Fn(String) + 'static,
{
    let dialog = Dialog::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button(ok_label, ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let entry = Entry::new();
    entry.set_text(initial);

    content.append(&entry);

    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let text = entry.text().to_string();
            on_accept(text.trim().to_string());
        }

        dialog.close();
    });

    dialog.present();
}

pub fn show_error(parent: &ApplicationWindow, message: &str) {
    let dialog = Dialog::builder()
        .title("Error")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("OK", ResponseType::Close);

    let label = Label::new(Some(message));
    label.set_wrap(true);

    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);

    dialog.content_area().append(&label);

    dialog.connect_response(|dialog, _| {
        dialog.close();
    });

    dialog.present();
}
