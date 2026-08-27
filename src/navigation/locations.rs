use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn desktop_dir() -> PathBuf {
    xdg_user_dir("DESKTOP", "Desktop")
}

pub fn documents_dir() -> PathBuf {
    xdg_user_dir("DOCUMENTS", "Documents")
}

pub fn downloads_dir() -> PathBuf {
    xdg_user_dir("DOWNLOAD", "Downloads")
}

pub fn music_dir() -> PathBuf {
    xdg_user_dir("MUSIC", "Music")
}

pub fn pictures_dir() -> PathBuf {
    xdg_user_dir("PICTURES", "Pictures")
}

pub fn videos_dir() -> PathBuf {
    xdg_user_dir("VIDEOS", "Videos")
}

pub fn public_dir() -> PathBuf {
    xdg_user_dir("PUBLICSHARE", "Public")
}

fn xdg_user_dir(env_var: &str, fallback: &str) -> PathBuf {
    let env_name = format!("XDG_{env_var}_DIR");

    if let Some(value) = std::env::var_os(env_name) {
        return PathBuf::from(value);
    }

    home_dir().join(fallback)
}

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
