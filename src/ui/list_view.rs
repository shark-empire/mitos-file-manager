use gtk::prelude::*;

use crate::filesystem::access;
use crate::i18n::tr;
use crate::ui::item_object::ItemObject;
use crate::util::{get_obj_data, set_obj_data};

pub fn create_list_view(selection: &gtk::MultiSelection) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection.clone()));
    view.set_show_column_separators(true);

    // ------------------------------------------------------------
    // Name column (icon + label)
    // ------------------------------------------------------------

    let name_factory = gtk::SignalListItemFactory::new();

    name_factory.connect_setup(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_margin_start(4);

        let icon = gtk::Image::new();
        let label = gtk::Label::new(None);
        label.set_halign(gtk::Align::Start);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);

        // Shown only for an item you can't write to -- see the bind
        // closure below and `filesystem::access::can_write`.
        let lock_icon = gtk::Image::from_icon_name("changes-prevent-symbolic");
        lock_icon.set_pixel_size(14);
        lock_icon.set_visible(false);
        lock_icon.add_css_class("dim-label");

        row.append(&icon);
        row.append(&label);
        row.append(&lock_icon);

        list_item.set_child(Some(&row));
        set_obj_data(list_item, "icon", icon);
        set_obj_data(list_item, "label", label);
        set_obj_data(list_item, "lock-icon", lock_icon);
    });

    name_factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

        let Some(obj) = list_item
            .item()
            .and_then(|o| o.downcast::<ItemObject>().ok())
        else {
            return;
        };

        let icon: Option<gtk::Image> = get_obj_data(list_item, "icon");
        let label: Option<gtk::Label> = get_obj_data(list_item, "label");
        let lock_icon: Option<gtk::Image> = get_obj_data(list_item, "lock-icon");

        if let (Some(icon), Some(label)) = (icon, label) {
            icon.set_icon_name(Some(&obj.icon_name()));
            label.set_label(&obj.name());
        }

        if let Some(lock_icon) = lock_icon {
            // One `access(2)` call, only for rows actually on screen (list
            // views virtualize -- bind only fires for visible rows).
            let locked = !access::can_write(&obj.get_path());
            lock_icon.set_visible(locked);

            if locked {
                lock_icon.set_tooltip_text(Some(&tr(
                    "Read-only \u{2014} you don't have permission to change this",
                )));
            }
        }
    });

    let name_sorter = gtk::CustomSorter::new(|a, b| {
        let a = a
            .downcast_ref::<ItemObject>()
            .map(|o| o.name().to_lowercase())
            .unwrap_or_default();
        let b = b
            .downcast_ref::<ItemObject>()
            .map(|o| o.name().to_lowercase())
            .unwrap_or_default();
        a.cmp(&b).into()
    });

    let name_column = gtk::ColumnViewColumn::new(Some(&tr("Name")), Some(name_factory));
    name_column.set_sorter(Some(&name_sorter));
    name_column.set_resizable(true);
    name_column.set_expand(true);
    view.append_column(&name_column);

    // ------------------------------------------------------------
    // Size / Type / Modified columns
    // ------------------------------------------------------------

    view.append_column(&text_column(
        &tr("Size"),
        |o| o.size_str(),
        |a, b| a.size().cmp(&b.size()),
    ));

    view.append_column(&text_column(
        &tr("Type"),
        |o| o.mime_type(),
        |a, b| a.mime_type().cmp(&b.mime_type()),
    ));

    view.append_column(&text_column(
        &tr("Modified"),
        |o| o.modified_str(),
        |a, b| a.modified_secs().cmp(&b.modified_secs()),
    ));

    view
}

fn text_column<F, S>(title: &str, text: F, cmp: S) -> gtk::ColumnViewColumn
where
    F: Fn(&ItemObject) -> String + 'static,
    S: Fn(&ItemObject, &ItemObject) -> std::cmp::Ordering + 'static,
{
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::new(None);
        label.set_halign(gtk::Align::Start);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        list_item.set_child(Some(&label));
    });

    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

        let Some(obj) = list_item
            .item()
            .and_then(|o| o.downcast::<ItemObject>().ok())
        else {
            return;
        };

        if let Some(label) = list_item
            .child()
            .and_then(|c| c.downcast::<gtk::Label>().ok())
        {
            label.set_label(&text(&obj));
        }
    });

    let sorter = gtk::CustomSorter::new(move |a, b| {
        match (
            a.downcast_ref::<ItemObject>(),
            b.downcast_ref::<ItemObject>(),
        ) {
            (Some(a), Some(b)) => cmp(a, b).into(),
            _ => std::cmp::Ordering::Equal.into(),
        }
    });

    let column = gtk::ColumnViewColumn::new(Some(title), Some(factory));

    column.set_sorter(Some(&sorter));
    column.set_resizable(true);

    column
}
