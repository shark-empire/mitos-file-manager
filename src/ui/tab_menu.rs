use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Notebook, Orientation, Popover};

pub fn show_tab_context_menu(notebook: &Notebook, page_widget: &gtk::Widget, x: f64, y: f64) {
    let popover = Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(notebook);
    popover.set_pointing_to(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1));

    let menu_box = GtkBox::new(Orientation::Vertical, 4);
    menu_box.set_margin_top(4);
    menu_box.set_margin_bottom(4);
    menu_box.set_margin_start(4);
    menu_box.set_margin_end(4);

    let close_btn = Button::with_label("Close Tab");
    let close_others_btn = Button::with_label("Close Other Tabs");
    let close_all_btn = Button::with_label("Close All Tabs");

    close_btn.set_has_frame(false);
    close_others_btn.set_has_frame(false);
    close_all_btn.set_has_frame(false);

    menu_box.append(&close_btn);
    menu_box.append(&close_others_btn);
    menu_box.append(&close_all_btn);

    popover.set_child(Some(&menu_box));

    let notebook_clone = notebook.clone();
    let page_widget_clone = page_widget.clone();

    close_btn.connect_clicked(move |_| {
        if let Some(page_num) = notebook_clone.page_num(&page_widget_clone) {
            notebook_clone.remove_page(page_num);
        }
        popover.popdown();
    });

    let notebook_clone2 = notebook.clone();
    let page_widget_clone2 = page_widget.clone();

    close_others_btn.connect_clicked(move |_| {
        let keep_page = notebook_clone2.page_num(&page_widget_clone2);

        let mut i = 0;
        while i < notebook_clone2.n_pages() {
            if Some(i) != keep_page {
                notebook_clone2.remove_page(i);
            } else {
                i += 1;
            }
        }

        popover.popdown();
    });

    let notebook_clone3 = notebook.clone();

    close_all_btn.connect_clicked(move |_| {
        while notebook_clone3.n_pages() > 0 {
            notebook_clone3.remove_page(0);
        }

        popover.popdown();
    });

    popover.popup();
}
