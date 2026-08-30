use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

#[derive(Clone)]
pub struct TrashItem {
    pub trash_name: String,
    pub original_path: PathBuf,
    pub file_path: PathBuf,
    pub info_path: PathBuf,
}

pub fn list() -> Vec<TrashItem> {
    let Some(root) = trash_root() else {
        return Vec::new();
    };

    let info_dir = root.join("info");
    let files_dir = root.join("files");

    let Ok(entries) = fs::read_dir(info_dir) else {
        return Vec::new();
    };

    let mut items = Vec::new();

    for entry in entries.flatten() {
        let info_path = entry.path();

        let is_trashinfo = info_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext == "trashinfo")
            .unwrap_or(false);

        if !is_trashinfo {
            continue;
        }

        let Some(trash_name) = info_path
            .file_stem()
            .map(|name| name.to_string_lossy().to_string())
        else {
            continue;
        };

        let file_path = files_dir.join(&trash_name);

        if !file_path.exists() {
            continue;
        }

        let original_path = parse_original_path(&info_path).unwrap_or_else(|| file_path.clone());

        items.push(TrashItem {
            trash_name,
            original_path,
            file_path,
            info_path,
        });
    }

    items.sort_by(|a, b| a.trash_name.cmp(&b.trash_name));

    items
}

pub fn restore(item: &TrashItem) -> io::Result<()> {
    if item.original_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "The original file already exists",
        ));
    }

    if let Some(parent) = item.original_path.parent() {
        fs::create_dir_all(parent)?;
    }

    move_with_fallback(&item.file_path, &item.original_path)?;

    if item.info_path.exists() {
        fs::remove_file(&item.info_path)?;
    }

    Ok(())
}

pub fn empty() -> io::Result<()> {
    let Some(root) = trash_root() else {
        return Ok(());
    };

    let files_dir = root.join("files");
    let info_dir = root.join("info");

    if files_dir.exists() {
        for entry in fs::read_dir(&files_dir)? {
            let entry = entry?;
            remove_any(&entry.path())?;
        }
    }

    if info_dir.exists() {
        for entry in fs::read_dir(&info_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                fs::remove_file(path)?;
            }
        }
    }

    Ok(())
}

fn trash_root() -> Option<PathBuf> {
    let data_home = data_home()?;
    Some(data_home.join("Trash"))
}

fn data_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(value);

        if path.is_absolute() {
            return Some(path);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".local/share"))
}

fn parse_original_path(info_path: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(info_path).ok()?;

    for line in content.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("Path=") {
            let rest = rest.trim();
            let rest = rest.strip_prefix("file://").unwrap_or(rest);

            return Some(PathBuf::from(percent_decode(rest)));
        }
    }

    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }

        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).to_string()
}

fn move_with_fallback(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_any(source, destination)?;
                remove_any(source)?;
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn copy_any(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir(source, destination)
    } else {
        fs::copy(source, destination)?;
        Ok(())
    }
}

fn copy_dir(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())?;

            #[cfg(unix)]
            symlink_unix(link_target, &target)?;

            #[cfg(not(unix))]
            {
                let _ = link_target;
            }
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

fn remove_any(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
