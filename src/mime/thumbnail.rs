use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::path::{Path, PathBuf};

pub fn thumbnail_path_for(path: &Path, mime: &str, size: u64) -> String {
    if !crate::config::settings::thumbnails_enabled() {
        return String::new();
    }

    if let Some(existing) = freedesktop_thumbnail_path(path) {
        return existing.to_string_lossy().to_string();
    }

    let max_size = crate::config::settings::thumbnail_max_bytes();

    if size > 0 && size <= max_size && mime.starts_with("image/") {
        return path.to_string_lossy().to_string();
    }

    String::new()
}

fn freedesktop_thumbnail_path(path: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let hash = thumbnail_hash(path)?;
    let cache = home.join(".cache/thumbnails");

    for directory in ["large", "normal"] {
        let candidate = cache.join(directory).join(format!("{hash}.png"));

        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// The freedesktop thumbnail cache key for `path`: an MD5 of its file://
/// URI, shared by the read side above and the video-thumbnail writer below
/// so both agree on where a given file's cached thumbnail lives.
fn thumbnail_hash(path: &Path) -> Option<String> {
    let file = gio::File::for_path(path);
    let uri = file.uri().to_string();
    // `.to_string()` regardless of whether this returns `String` or
    // `GString` -- both are `Display`, so this is safe either way.
    glib::compute_checksum_for_string(glib::ChecksumType::Md5, &uri).map(|h| h.to_string())
}

/// Where a generated thumbnail for `path` should be written -- the
/// "normal" (128px) tier of the same cache `freedesktop_thumbnail_path`
/// reads from, so it's picked up by this app *and* any other freedesktop-
/// compliant file manager without extra wiring.
fn freedesktop_cache_target(path: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let hash = thumbnail_hash(path)?;

    Some(
        home.join(".cache/thumbnails/normal")
            .join(format!("{hash}.png")),
    )
}

/// Kick off a background job that tries to generate a thumbnail for a
/// video file by grabbing one frame with `ffmpeg`, caching it under the
/// same freedesktop thumbnail directory images already use. `read_items`
/// runs on the main thread and can't afford to shell out per video, so
/// this hands the work to a background thread and hands back a receiver
/// instead -- consume it with `glib::MainContext::default().spawn_local`,
/// the same pattern already used for the config and directory watchers in
/// `main.rs`. Yields `Some(cache_path)` on success, `None` if `ffmpeg`
/// isn't installed, thumbnails are disabled, or generation failed --
/// callers should treat `None` as "keep showing the generic icon", not as
/// an error worth surfacing to the user.
pub fn spawn_video_thumbnail_job(
    path: PathBuf,
    mime: String,
) -> async_channel::Receiver<Option<String>> {
    let (tx, rx) = async_channel::bounded(1);

    if !crate::config::settings::thumbnails_enabled() || !mime.starts_with("video/") {
        let _ = tx.send_blocking(None);
        return rx;
    }

    std::thread::spawn(move || {
        let result = generate_video_thumbnail(&path);
        let _ = tx.send_blocking(result);
    });

    rx
}

fn generate_video_thumbnail(path: &Path) -> Option<String> {
    let target = freedesktop_cache_target(path)?;

    if target.exists() {
        return Some(target.to_string_lossy().to_string());
    }

    let parent = target.parent()?;
    std::fs::create_dir_all(parent).ok()?;

    // Render beside the real target first, then rename into place, so a
    // reader (including this app, on the next folder visit) never sees a
    // half-written PNG if ffmpeg is killed mid-frame.
    let tmp_target = parent.join(format!(
        "{}.tmp-{}",
        target.file_name()?.to_string_lossy(),
        std::process::id()
    ));

    // `-ss` before `-i` seeks the input directly to that timestamp instead
    // of decoding everything up to it -- much cheaper for a thumbnail.
    let spawned = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-ss", "00:00:01", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale=256:-1"])
        .arg(&tmp_target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let Ok(status) = spawned else {
        // ffmpeg isn't installed / couldn't be spawned. Not an error the
        // user needs a dialog for -- the item just keeps its generic icon.
        return None;
    };

    if !status.success() || !tmp_target.exists() {
        let _ = std::fs::remove_file(&tmp_target);
        return None;
    }

    std::fs::rename(&tmp_target, &target).ok()?;
    Some(target.to_string_lossy().to_string())
}
