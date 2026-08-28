use gtk::gio;
use gtk::gio::prelude::*;
use std::path::Path;

/// Get all applications that can handle a given MIME type.
pub fn apps_for_mime(mime: &str) -> Vec<gio::AppInfo> {
    gio::AppInfo::all_for_type(mime)
}

/// Get the default application for a MIME type.
pub fn default_app_for_mime(mime: &str) -> Option<gio::AppInfo> {
    gio::AppInfo::default_for_type(mime).ok()
}

/// Set an application as the default for a MIME type.
pub fn set_default_app(app: &gio::AppInfo, mime: &str) -> Result<(), String> {
    app.set_as_default_for_type(mime)
        .map_err(|e| e.to_string())
}

/// Launch an application with a file.
pub fn launch_app_with_file(app: &gio::AppInfo, path: &Path) -> Result<(), String> {
    let file = gio::File::for_path(path);
    app.launch(&[&file], None::<&gio::AppLaunchContext>)
        .map_err(|e| e.to_string())
}

/// Get a display-friendly list of (app_name, app_info) pairs.
pub fn app_display_names(apps: &[gio::AppInfo]) -> Vec<(String, gio::AppInfo)> {
    apps.iter()
        .filter_map(|app| {
            let name = app.display_name();
            if name.is_empty() {
                None
            } else {
                Some((name, app.clone()))
            }
        })
        .collect()
}
