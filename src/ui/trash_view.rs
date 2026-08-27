use crate::filesystem::trash;
use crate::ui::dialogs;
use gtk::glib;
use gtk::prelude::*;
use gtk::{
    ApplicationWindow, Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation,
    ResponseType, ScrolledWindow, SelectionMode,
};
use std::cell::Cell;
use std::rc::Rc;

pub fn show(parent: &ApplicationWindow, refresh_main: Rc<dyn Fn()>) {
    let window = gtk::Window::builder()
        .title("Trash")
        .transient_for(parent)
        .default_width(700)
        .default_height(480)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 6);

    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let toolbar = GtkBox::new(Orientation::Horizontal, 6);

    let refresh_btn = Button::with_label("Refresh");
    let empty_btn = Button::with_label("Empty Trash");

    toolbar.append(&refresh_btn);
    toolbar.append(&empty_btn);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::None);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();

    scrolled.set_child(Some(&list));
    scrolled.set_vexpand(true);

    root.append(&toolbar);
    root.append(&scrolled);

    window.set_child(Some(&root));

    populate(&list, &window, refresh_main.clone());

    {
        let list = list.clone();
        let window = window.clone();
        let refresh_main = refresh_main.clone();

        refresh_btn.connect_clicked(move |_| {
            populate(&list, &window, refresh_main.clone());
        });
    }

    {
        let list = list.clone();
        let window = window.clone();
        let refresh_main = refresh_main.clone();

        empty_btn.connect_clicked(move |_| {
            if !confirm_empty(&window) {
                return;
            }

            if let Err(err) = trash::empty() {
                dialogs::show_error(&window, &format!("Could not empty trash: {err}"));
            }

            populate(&list, &window, refresh_main.clone());
            refresh_main();
        });
    }

    window.present();
}

fn populate(list: &ListBox, parent: &ApplicationWindow, refresh_main: Rc<dyn Fn()>) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }

    let items = trash::list();

    if items.is_empty() {
        let row = ListBoxRow::new();
        let label = Label::new(Some("Trash is empty"));

        row.set_child(Some(&label));
        list.append(&row);

        return;
    }

    for item in items {
        let row = ListBoxRow::new();

        let row_box = GtkBox::new(Orientation::Horizontal, 6);

        row_box.set_margin_top(4);
        row_box.set_margin_bottom(4);
        row_box.set_margin_start(6);
        row_box.set_margin_end(6);

        let label_text = format!(
            "{}\nOriginal location: {}",
            item.trash_name,
            item.original_path.display()
        );

        let label = Label::new(Some(&label_text));
        label.set_wrap(true);
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);

        let restore_btn = Button::with_label("Restore");

        let item = item.clone();
        let list = list.clone();
        let parent = parent.clone();
        let refresh_main = refresh_main.clone();

        restore_btn.connect_clicked(move |_| {
            if let Err(err) = trash::restore(&item) {
                dialogs::show_error(&parent, &format!("Could not restore item: {err}"));
            }

            populate(&list, &parent, refresh_main.clone());
            refresh_main();
        });

        row_box.append(&label);
        row_box.append(&restore_btn);

        row.set_child(Some(&row_box));
        list.append(&row);
    }
}

fn confirm_empty(parent: &ApplicationWindow) -> bool {
    let dialog = gtk::Dialog::builder()
        .title("Empty Trash")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Empty Trash", ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some(
        "All items in the trash will be permanently deleted.\n\nContinue?",
    ));

    label.set_wrap(true);
    content.append(&label);

    let loop_ = glib::MainLoop::new(None, false);
    let result = Rc::new(Cell::new(false));

    let result_clone = result.clone();
    let loop_clone = loop_.clone();

    dialog.connect_response(move |dialog, response| {
        result_clone.set(response == ResponseType::Accept);
        dialog.close();
        loop_clone.quit();
    });

    let loop_close = loop_.clone();

    dialog.connect_close_request(move |_| {
        loop_close.quit();
        gtk::Inhibit(false)
    });

    dialog.present();
    loop_.run();

    result.get()
}
