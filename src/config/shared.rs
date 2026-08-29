use std::fs;
use std::io;
use std::path::PathBuf;

/// Shared MITOS desktop configuration.
///
/// This file is read by:
/// - mitos-gui (compositor) - for theme, wallpaper, shell layout
/// - MITOS Files - for theme mode, thumbnails, etc.
/// - Future MITOS apps
///
/// Location: ~/.config/mitos/home.conf
#[derive(Debug, Clone)]
pub struct SharedConfig {
    pub theme_mode: String,
    pub accent_color: String,
    pub glass_opacity: f32,
    pub panel_radius: f32,
    pub wallpaper: String,
    pub show_hidden_files: bool,
    pub enable_thumbnails: bool,
    pub thumbnail_max_mb: u64,
}

impl Default for SharedConfig {
    fn default() -> Self {
        Self {
            theme_mode: "light".to_string(),
            accent_color: "#4d9eff".to_string(),
            glass_opacity: 0.72,
            panel_radius: 18.0,
            wallpaper: String::new(),
            show_hidden_files: false,
            enable_thumbnails: true,
            thumbnail_max_mb: 50,
        }
    }
}

impl SharedConfig {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };

        let Ok(contents) = fs::read_to_string(&path) else {
            return Self::default();
        };

        let mut config = Self::default();

        for line in contents.lines() {
            let line = line.trim();
            
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            let key = key.trim();
            let value = value.trim();

            match key {
                "theme_mode" => config.theme_mode = value.to_string(),
                "accent_color" => config.accent_color = value.to_string(),
                "glass_opacity" => {
                    if let Ok(v) = value.parse() {
                        config.glass_opacity = v;
                    }
                }
                "panel_radius" => {
                    if let Ok(v) = value.parse() {
                        config.panel_radius = v;
                    }
                }
                "wallpaper" => config.wallpaper = value.to_string(),
                "show_hidden_files" => {
                    config.show_hidden_files = value == "true";
                }
                "enable_thumbnails" => {
                    config.enable_thumbnails = value == "true";
                }
                "thumbnail_max_mb" => {
                    if let Ok(v) = value.parse() {
                        config.thumbnail_max_mb = v;
                    }
                }
                _ => {}
            }
        }

        config
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "No config directory")
        })?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = format!(
            r#"# MITOS Desktop Configuration
# Shared by mitos-gui (compositor) and MITOS applications

# Theme mode: light or dark
theme_mode = {}

# Accent color (hex format)
accent_color = {}

# Glass panel transparency (0.0 to 1.0)
glass_opacity = {:.2}

# Panel corner radius
panel_radius = {:.1}

# Wallpaper path (empty for default)
wallpaper = {}

# File manager: show hidden files
show_hidden_files = {}

# File manager: enable thumbnails
enable_thumbnails = {}

# File manager: max thumbnail size in MB
thumbnail_max_mb = {}
"#,
            self.theme_mode,
            self.accent_color,
            self.glass_opacity,
            self.panel_radius,
            self.wallpaper,
            self.show_hidden_files,
            self.enable_thumbnails,
            self.thumbnail_max_mb,
        );

        fs::write(path, contents)
    }
}

fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mitos").join("home.conf"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".config").join("mitos").join("home.conf"))
}
