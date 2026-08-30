use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::shared::SharedConfig;

static SHOW_HIDDEN: AtomicBool = AtomicBool::new(false);
static THUMBNAILS_ENABLED: AtomicBool = AtomicBool::new(true);
static THUMBNAIL_MAX_MB: AtomicU64 = AtomicU64::new(50);
static CONFIRM_TRASH: AtomicBool = AtomicBool::new(true);
static THEME_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub show_hidden_files: bool,
    pub enable_thumbnails: bool,
    pub thumbnail_max_mb: u64,
    pub confirm_trash: bool,
    pub theme_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_hidden_files: false,
            enable_thumbnails: true,
            thumbnail_max_mb: 50,
            confirm_trash: true,
            theme_mode: "light".to_string(),
        }
    }
}

pub fn load() -> Settings {
    let settings = read_from_disk().unwrap_or_default();

    // Also load from shared config
    let shared = SharedConfig::load();

    let merged = Settings {
        show_hidden_files: shared.show_hidden_files,
        enable_thumbnails: shared.enable_thumbnails,
        thumbnail_max_mb: shared.thumbnail_max_mb,
        confirm_trash: settings.confirm_trash,
        theme_mode: shared.theme_mode,
    };

    apply_globals(&merged);
    merged
}

pub fn current() -> Settings {
    Settings {
        show_hidden_files: show_hidden_default(),
        enable_thumbnails: thumbnails_enabled(),
        thumbnail_max_mb: thumbnail_max_mb(),
        confirm_trash: confirm_trash_enabled(),
    }
}

pub fn apply_and_save(
    show_hidden_files: bool,
    enable_thumbnails: bool,
    thumbnail_max_mb: u64,
    confirm_trash: bool,
    theme_mode: &str,
) {
    let settings = Settings {
        show_hidden_files,
        enable_thumbnails,
        thumbnail_max_mb,
        confirm_trash,
        theme_mode: theme_mode.to_string(),
    };

    apply_globals(&settings);

    // Save to MITOS Files config
    let _ = write_to_disk(&settings);

    // Sync to shared MITOS config (for compositor)
    let mut shared = SharedConfig::load();
    shared.show_hidden_files = show_hidden_files;
    shared.enable_thumbnails = enable_thumbnails;
    shared.thumbnail_max_mb = thumbnail_max_mb;
    shared.theme_mode = theme_mode.to_string();
    let _ = shared.save();
}

pub fn set_show_hidden(value: bool) {
    SHOW_HIDDEN.store(value, Ordering::Relaxed);
    save_current();
}

pub fn show_hidden_default() -> bool {
    SHOW_HIDDEN.load(Ordering::Relaxed)
}

pub fn thumbnails_enabled() -> bool {
    THUMBNAILS_ENABLED.load(Ordering::Relaxed)
}

pub fn thumbnail_max_mb() -> u64 {
    THUMBNAIL_MAX_MB.load(Ordering::Relaxed)
}

pub fn thumbnail_max_bytes() -> u64 {
    thumbnail_max_mb().saturating_mul(1024 * 1024)
}

pub fn confirm_trash_enabled() -> bool {
    CONFIRM_TRASH.load(Ordering::Relaxed)
}

fn apply_globals(settings: &Settings) {
    SHOW_HIDDEN.store(settings.show_hidden_files, Ordering::Relaxed);
    THUMBNAILS_ENABLED.store(settings.enable_thumbnails, Ordering::Relaxed);
    THUMBNAIL_MAX_MB.store(settings.thumbnail_max_mb, Ordering::Relaxed);
    CONFIRM_TRASH.store(settings.confirm_trash, Ordering::Relaxed);

    let dark = settings.theme_mode == "dark";
    THEME_MODE.store(dark, Ordering::Relaxed);
}

fn save_current() {
    let _ = write_to_disk(&current());
}

fn settings_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    Some(config_dir.join("mitos/file-manager/settings.json"))
}

fn read_from_disk() -> Option<Settings> {
    let path = settings_path()?;
    let data = fs::read_to_string(path).ok()?;

    serde_json::from_str(&data).ok()
}

fn write_to_disk(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine config directory",
        )
    })?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(settings)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;

    fs::write(path, json)
}

pub fn theme_mode() -> crate::ui::theme::ThemeMode {
    if THEME_MODE.load(Ordering::Relaxed) {
        crate::ui::theme::ThemeMode::Dark
    } else {
        crate::ui::theme::ThemeMode::Light
    }
}

pub fn is_dark_theme() -> bool {
    THEME_MODE.load(Ordering::Relaxed)
}
