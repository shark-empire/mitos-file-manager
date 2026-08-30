use gtk::prelude::*;

pub fn setup_widget_accessibility(window: &gtk::ApplicationWindow) {
    // Set accessible role and name for the main window
    window.set_tooltip_text(Some("MITOS File Manager - Browse and manage files"));
}

pub fn make_button_accessible(button: &gtk::Button, description: &str) {
    button.set_tooltip_text(Some(description));
}

pub fn make_entry_accessible(entry: &gtk::Entry, label: &str) {
    entry.set_tooltip_text(Some(label));
}

pub fn setup_keyboard_help() -> Vec<(String, String)> {
    vec![
        ("Ctrl+C".to_string(), "Copy selected files".to_string()),
        ("Ctrl+X".to_string(), "Cut selected files".to_string()),
        ("Ctrl+V".to_string(), "Paste files".to_string()),
        ("Ctrl+T".to_string(), "New tab".to_string()),
        ("Ctrl+W".to_string(), "Close tab".to_string()),
        ("Ctrl+H".to_string(), "Toggle hidden files".to_string()),
        ("Ctrl+L".to_string(), "Focus location entry".to_string()),
        ("Alt+Left".to_string(), "Navigate back".to_string()),
        ("Alt+Right".to_string(), "Navigate forward".to_string()),
        ("F2".to_string(), "Rename selected file".to_string()),
        ("F5".to_string(), "Refresh current view".to_string()),
        ("Delete".to_string(), "Move to trash".to_string()),
        (
            "Enter".to_string(),
            "Open selected item / Execute search".to_string(),
        ),
    ]
}
