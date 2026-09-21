use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

// Each user folder is looked up the same three ways, most specific first:
//   1. an explicit `XDG_<NAME>_DIR` environment override,
//   2. the user's own `~/.config/user-dirs.dirs` (which is where the
//      desktop actually records them -- those variables are almost never
//      exported into the environment; the `dirs` crate reads that file), and
//   3. `~/<Name>` as a last resort.

pub fn desktop_dir() -> PathBuf {
    xdg_user_dir("DESKTOP", dirs::desktop_dir(), "Desktop")
}

pub fn documents_dir() -> PathBuf {
    xdg_user_dir("DOCUMENTS", dirs::document_dir(), "Documents")
}

pub fn downloads_dir() -> PathBuf {
    xdg_user_dir("DOWNLOAD", dirs::download_dir(), "Downloads")
}

pub fn music_dir() -> PathBuf {
    xdg_user_dir("MUSIC", dirs::audio_dir(), "Music")
}

pub fn pictures_dir() -> PathBuf {
    xdg_user_dir("PICTURES", dirs::picture_dir(), "Pictures")
}

pub fn videos_dir() -> PathBuf {
    xdg_user_dir("VIDEOS", dirs::video_dir(), "Videos")
}

pub fn public_dir() -> PathBuf {
    xdg_user_dir("PUBLICSHARE", dirs::public_dir(), "Public")
}

fn xdg_user_dir(env_var: &str, configured: Option<PathBuf>, fallback: &str) -> PathBuf {
    let env_name = format!("XDG_{env_var}_DIR");

    if let Some(value) = std::env::var_os(env_name) {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }

    if let Some(dir) = configured {
        return dir;
    }

    home_dir().join(fallback)
}

/// The sidebar's "Places" list: a display name and where it goes.
pub fn default_places() -> Vec<(String, PathBuf)> {
    vec![
        ("Home".to_string(), home_dir()),
        ("Desktop".to_string(), desktop_dir()),
        ("Documents".to_string(), documents_dir()),
        ("Downloads".to_string(), downloads_dir()),
        ("Music".to_string(), music_dir()),
        ("Pictures".to_string(), pictures_dir()),
        ("Videos".to_string(), videos_dir()),
        ("Public".to_string(), public_dir()),
        ("Computer".to_string(), PathBuf::from("/")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_places_start_at_home_and_end_at_the_filesystem_root() {
        let places = default_places();

        assert_eq!(places.first().map(|(name, _)| name.as_str()), Some("Home"));
        assert_eq!(
            places.last().map(|(_, path)| path.clone()),
            Some(PathBuf::from("/"))
        );
        assert_eq!(places.len(), 9);
    }

    #[test]
    fn every_user_folder_resolves_to_an_absolute_path() {
        for dir in [
            desktop_dir(),
            documents_dir(),
            downloads_dir(),
            music_dir(),
            pictures_dir(),
            videos_dir(),
            public_dir(),
        ] {
            assert!(dir.is_absolute(), "{dir:?} should be absolute");
        }
    }
}
