use gtk::glib;
use gtk::prelude::*;

use crate::filesystem::directory::Item;
use crate::filesystem::metadata;

mod imp {
    use super::*;
    use glib::Properties;
    use std::cell::RefCell;

    #[derive(Debug, Default, Properties)]
    #[properties(wrapper_type = super::ItemObject)]
    pub struct ItemObject {
        #[property(get, set)]
        pub name: RefCell<String>,

        #[property(get, set)]
        pub path: RefCell<String>,

        #[property(get, set)]
        pub is_dir: RefCell<bool>,

        #[property(get, set)]
        pub mime_type: RefCell<String>,

        #[property(get, set)]
        pub icon_name: RefCell<String>,

        #[property(get, set)]
        pub thumbnail_path: RefCell<String>,

        #[property(get, set)]
        pub size_str: RefCell<String>,

        #[property(get, set)]
        pub modified_str: RefCell<String>,

        #[property(get, set)]
        pub permissions: RefCell<String>,

        #[property(get, set)]
        pub is_symlink: RefCell<bool>,

       #[property(get, set)]
        pub size: RefCell<u64>,

    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemObject {
        const NAME: &'static str = "MitosFileItem";
        type Type = super::ItemObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ItemObject {}
}

glib::wrapper! {
    pub struct ItemObject(ObjectSubclass<imp::ItemObject>);
}

impl ItemObject {
    pub fn new(item: &Item) -> Self {
        let size_str = if item.is_dir {
            "-".to_string()
        } else {
            metadata::format_size(item.metadata.size)
        };

        let modified_str = metadata::format_modified(item.metadata.modified);

        glib::Object::builder()
            .property("name", &item.name)
            .property("path", item.path.to_string_lossy().to_string())
            .property("is-dir", item.is_dir)
            .property("mime-type", &item.mime)
            .property("icon-name", &item.icon_name)
            .property("thumbnail-path", &item.thumbnail_path)
            .property("size-str", size_str)
            .property("modified-str", modified_str)
            .property("permissions", &item.metadata.permissions)
            .property("is-symlink", item.metadata.is_symlink)
            .property("size", item.metadata.size)
            .build()
    }

    pub fn get_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(self.path())
    }
}
