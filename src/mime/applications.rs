use gtk::gio;
use gtk::gio::prelude::*;
use std::path::Path;

/// Get all applications that can handle a given MIME type.
pub fn apps_for_mime(mime: &str) -> Vec<gio::AppInfo> {
    gio::AppInfo::all_for_type(mime)
}

/// Get the default application for a MIME type.
pub fn default_app_for_mime(mime: &str) -> Option<gio::AppInfo> {
    gio::AppInfo::default_for_type(mime, false)
}

/// Set an application as the default for a MIME type.
pub fn set_default_app(app: &gio::AppInfo, mime: &str) -> Result<(), String> {
    app.set_as_default_for_type(mime).map_err(|e| e.to_string())
}

/// Launch an application with a file.
pub fn launch_app_with_file(app: &gio::AppInfo, path: &Path) -> Result<(), String> {
    let file = gio::File::for_path(path);

    app.launch(&[file], None::<&gio::AppLaunchContext>)
        .map_err(|e| e.to_string())?;

    // Opening something puts it in the recent-files list.
    crate::navigation::recent::record(path);

    Ok(())
}

/// Get a display-friendly list of (app_name, app_info) pairs.
pub fn app_display_names(apps: &[gio::AppInfo]) -> Vec<(String, gio::AppInfo)> {
    apps.iter()
        .filter_map(|app| {
            let name = app.display_name();
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), app.clone()))
            }
        })
        .collect()
}

// ============================================================================
// RECOMMENDED DEFAULTS
//
// A one-click alternative to picking a default app per MIME type by hand
// through "Open With". Everything here is just a curated list of (which
// app, which MIME types) fed through `set_default_app` above -- no new
// mechanism, same one the per-file "Open With" tab already uses.
// ============================================================================

struct DefaultAppGroup {
    label: &'static str,
    // Tried in order; the first one actually installed wins. Letting
    // mpv and Celluloid share a list (rather than hardcoding one) means
    // this keeps working if only one of the two is on the system.
    desktop_ids: &'static [&'static str],
    mime_types: &'static [&'static str],
}

const RECOMMENDED_DEFAULTS: &[DefaultAppGroup] = &[
    DefaultAppGroup {
        label: "Video",
        desktop_ids: &[
            "mpv.desktop",
            "io.github.celluloid_player.Celluloid.desktop",
        ],
        mime_types: &[
            "video/mp4",
            "video/x-matroska",
            "video/webm",
            "video/quicktime",
            "video/x-msvideo",
            "video/mpeg",
            "video/x-flv",
            "video/3gpp",
            "video/3gpp2",
            "video/ogg",
            "video/x-ms-wmv",
            "video/mp2t",
        ],
    },
    DefaultAppGroup {
        label: "Audio",
        desktop_ids: &[
            "mpv.desktop",
            "io.github.celluloid_player.Celluloid.desktop",
        ],
        mime_types: &[
            "audio/mpeg",
            "audio/flac",
            "audio/x-flac",
            "audio/ogg",
            "audio/wav",
            "audio/x-wav",
            "audio/mp4",
            "audio/aac",
            "audio/x-ms-wma",
            "audio/opus",
            "audio/webm",
        ],
    },
    DefaultAppGroup {
        label: "Text",
        desktop_ids: &["org.gnome.TextEditor.desktop"],
        mime_types: &[
            "text/plain",
            "text/markdown",
            "text/x-rust",
            "text/x-python",
            "text/x-csrc",
            "text/x-c++src",
            "text/x-java",
            "text/x-shellscript",
            "application/json",
            "application/xml",
            "application/x-yaml",
            "text/x-toml",
            "text/csv",
            "text/x-log",
        ],
    },
];

/// What happened when trying to apply one `DefaultAppGroup`.
pub struct DefaultsOutcome {
    pub label: &'static str,
    /// `None` means none of that group's candidate apps are installed --
    /// nothing was changed for this category.
    pub app_name: Option<String>,
    pub applied: usize,
    pub failed: usize,
}

/// Set mpv/Celluloid as the default for common video and audio types, and
/// GNOME Text Editor for common text/code types -- across every MIME type
/// in `RECOMMENDED_DEFAULTS`, skipping (not erroring on) any category
/// whose app isn't installed. Returns one outcome per category so the
/// caller can show exactly what happened for each.
pub fn apply_recommended_defaults() -> Vec<DefaultsOutcome> {
    RECOMMENDED_DEFAULTS
        .iter()
        .map(|group| {
            let found = group
                .desktop_ids
                .iter()
                .find_map(|id| gio::DesktopAppInfo::new(id));

            let Some(desktop_app) = found else {
                return DefaultsOutcome {
                    label: group.label,
                    app_name: None,
                    applied: 0,
                    failed: group.mime_types.len(),
                };
            };

            let app_name = desktop_app.display_name().to_string();
            let app_info: gio::AppInfo = desktop_app.upcast();

            let mut applied = 0;
            let mut failed = 0;

            for mime in group.mime_types {
                match set_default_app(&app_info, mime) {
                    Ok(()) => applied += 1,
                    Err(_) => failed += 1,
                }
            }

            DefaultsOutcome {
                label: group.label,
                app_name: Some(app_name),
                applied,
                failed,
            }
        })
        .collect()
}
