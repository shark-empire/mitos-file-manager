use crate::i18n::tr;
use gtk::prelude::*;

pub fn setup_widget_accessibility(window: &gtk::ApplicationWindow) {
    // Set accessible role and name for the main window
    window.set_tooltip_text(Some(&tr("MITOS File Manager - Browse and manage files")));
}

/// Describe what a button-like widget (a `Button`, a `CheckButton`, a
/// dropdown, ...) does, as a tooltip -- the hint sighted mouse users get on
/// hover, and the description assistive tech falls back on for controls
/// that only have a short label.
pub fn make_button_accessible(button: &impl IsA<gtk::Widget>, description: &str) {
    button.set_tooltip_text(Some(description));
}

/// Same idea for text inputs (the location bar, the search box): say what
/// belongs in them.
pub fn make_entry_accessible(entry: &impl IsA<gtk::Widget>, label: &str) {
    entry.set_tooltip_text(Some(label));
}

/// Key combo, shown as typed (never translated -- it's the physical keys
/// to press), paired with what it does (translated).
pub fn setup_keyboard_help() -> Vec<(String, String)> {
    vec![
        ("Ctrl+C".to_string(), tr("Copy selected files")),
        ("Ctrl+X".to_string(), tr("Cut selected files")),
        ("Ctrl+V".to_string(), tr("Paste files")),
        ("Ctrl+D".to_string(), tr("Duplicate selected files")),
        (
            "Shift+Delete".to_string(),
            tr("Delete selected files permanently"),
        ),
        ("Ctrl+Z".to_string(), tr("Undo the last action")),
        ("Ctrl+Shift+Z".to_string(), tr("Redo the last undone action")),
        ("Ctrl+F".to_string(), tr("Search")),
        (
            "Alt+Up / Backspace".to_string(),
            tr("Go to the parent folder"),
        ),
        ("Ctrl+T".to_string(), tr("New tab")),
        ("Ctrl+W".to_string(), tr("Close tab")),
        ("Ctrl+H".to_string(), tr("Toggle hidden files")),
        ("Ctrl+L".to_string(), tr("Focus location entry")),
        ("Alt+Left".to_string(), tr("Navigate back")),
        ("Alt+Right".to_string(), tr("Navigate forward")),
        ("F2".to_string(), tr("Rename selected file")),
        ("F5".to_string(), tr("Refresh current view")),
        ("Delete".to_string(), tr("Move to trash")),
        (
            "Enter".to_string(),
            tr("Open selected item / Execute search"),
        ),
    ]
}
