use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

/// Longest side, in pixels, of a generated image thumbnail: the
/// freedesktop "normal" size, and more than the icon grid ever shows.
const THUMBNAIL_SIZE: i32 = 128;

/// Is a thumbnail worth looking up or generating for a file of this MIME
/// type and size?
pub fn wants_thumbnail(mime: &str, size: u64) -> bool {
    if !crate::config::settings::thumbnails_enabled() {
        return false;
    }

    if mime.starts_with("video/") {
        return true;
    }

    // Images bigger than the "max thumbnail image size" setting keep their
    // generic icon: decoding them is exactly the cost being avoided.
    mime.starts_with("image/") && size > 0 && size <= crate::config::settings::thumbnail_max_bytes()
}

/// The thumbnail for `path` if one is already in the shared freedesktop
/// cache and is at least as new as the file (empty string if not). This is
/// the cheap check done as a row scrolls into view; generating a missing
/// one is `request_thumbnail`'s job.
pub fn thumbnail_path_for(path: &Path) -> String {
    match freedesktop_thumbnail_path(path) {
        Some(cached) if is_fresh(&cached, path) => cached.to_string_lossy().to_string(),
        _ => String::new(),
    }
}

/// A cached thumbnail older than the file it was made from is stale (the
/// image was edited since); regenerate rather than show the old picture.
fn is_fresh(thumbnail: &Path, source: &Path) -> bool {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();

    match (modified(thumbnail), modified(source)) {
        (Some(thumb_time), Some(source_time)) => thumb_time >= source_time,
        // If either time can't be read, trust what's there.
        _ => true,
    }
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

struct ThumbnailJob {
    path: PathBuf,
    mime: String,
    /// Raised when the row that asked has scrolled away, so a fast scroll
    /// through thousands of photos doesn't decode every one it passed.
    cancel: Arc<AtomicBool>,
    reply: async_channel::Sender<Option<String>>,
}

/// The queue feeding the thumbnail workers, started on first use.
fn queue() -> &'static async_channel::Sender<ThumbnailJob> {
    static QUEUE: OnceLock<async_channel::Sender<ThumbnailJob>> = OnceLock::new();

    QUEUE.get_or_init(|| {
        let (sender, receiver) = async_channel::unbounded::<ThumbnailJob>();

        // Two workers: enough to keep a scrolling grid supplied, few enough
        // that a folder of ten thousand photos can't spawn ten thousand
        // decoders (or ten thousand ffmpeg processes).
        for _ in 0..2 {
            let receiver = receiver.clone();

            std::thread::spawn(move || {
                while let Ok(job) = receiver.recv_blocking() {
                    let result = if job.cancel.load(Ordering::Relaxed) {
                        None
                    } else if job.mime.starts_with("video/") {
                        generate_video_thumbnail(&job.path)
                    } else {
                        generate_image_thumbnail(&job.path)
                    };

                    let _ = job.reply.send_blocking(result);
                }
            });
        }

        sender
    })
}

/// Queue the generation of a thumbnail for `path` (images are scaled with
/// gdk-pixbuf, videos get one frame from `ffmpeg`) into the shared
/// freedesktop cache, and hand back a receiver for the result: consume it
/// with `glib::MainContext::default().spawn_local`. Yields
/// `Some(cache_path)` on success, `None` if it was cancelled, isn't
/// possible (no `ffmpeg`, a format nothing can decode) or failed -- callers
/// treat `None` as "keep the generic icon", never as an error to show.
pub fn request_thumbnail(
    path: PathBuf,
    mime: String,
    cancel: Arc<AtomicBool>,
) -> async_channel::Receiver<Option<String>> {
    let (reply, receiver) = async_channel::bounded(1);

    let _ = queue().send_blocking(ThumbnailJob {
        path,
        mime,
        cancel,
        reply,
    });

    receiver
}

/// Scale an image down to `THUMBNAIL_SIZE` and store it in the freedesktop
/// cache, tagged with the source's URI and modification time as the spec
/// requires -- so other file managers reuse it, and this one can tell when
/// it has gone stale.
///
/// Decoding straight to the small size (rather than loading the whole
/// picture and shrinking it) is what keeps a folder of large photos from
/// costing tens of megabytes per visible tile.
fn generate_image_thumbnail(path: &Path) -> Option<String> {
    let target = freedesktop_cache_target(path)?;

    if target.exists() && is_fresh(&target, path) {
        return Some(target.to_string_lossy().to_string());
    }

    let parent = target.parent()?;
    std::fs::create_dir_all(parent).ok()?;

    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, THUMBNAIL_SIZE, THUMBNAIL_SIZE, true).ok()?;
    // Phone photos are stored sideways with an "orientation" flag.
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);

    let uri = gio::File::for_path(path).uri().to_string();
    let modified = std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs()
        .to_string();

    // Write beside the target and rename into place, so nothing ever reads
    // a half-written PNG.
    let tmp_target = parent.join(format!(
        "{}.tmp-{}",
        target.file_name()?.to_string_lossy(),
        std::process::id()
    ));

    let saved = pixbuf.savev(
        &tmp_target,
        "png",
        &["tEXt::Thumb::URI", "tEXt::Thumb::MTime"],
        &[uri.as_str(), modified.as_str()],
    );

    if saved.is_err() {
        let _ = std::fs::remove_file(&tmp_target);
        return None;
    }

    std::fs::rename(&tmp_target, &target).ok()?;

    Some(target.to_string_lossy().to_string())
}

fn generate_video_thumbnail(path: &Path) -> Option<String> {
    let target = freedesktop_cache_target(path)?;

    if target.exists() && is_fresh(&target, path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn only_images_and_videos_are_worth_a_thumbnail() {
        // (Assumes thumbnails are enabled, which is the default.)
        assert!(wants_thumbnail("image/png", 1_000));
        assert!(wants_thumbnail("video/mp4", 10_000_000_000));
        assert!(!wants_thumbnail("text/plain", 1_000));
        assert!(!wants_thumbnail("inode/directory", 0));
        // An empty "image" has nothing to decode.
        assert!(!wants_thumbnail("image/png", 0));
    }

    #[test]
    fn a_thumbnail_older_than_its_source_is_stale() {
        let dir = scratch_dir("thumb-fresh");
        let thumb = dir.join("thumb.png");
        let source = dir.join("photo.png");

        std::fs::write(&thumb, "t").unwrap();
        std::fs::write(&source, "s").unwrap();

        // Set the times explicitly rather than sleeping and hoping the
        // filesystem's clock resolution is fine enough.
        let set_modified = |path: &Path, time: SystemTime| {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };

        let now = SystemTime::now();
        set_modified(&source, now);
        set_modified(&thumb, now - std::time::Duration::from_secs(100));
        assert!(!is_fresh(&thumb, &source));

        // Regenerated after the edit: current again.
        set_modified(&thumb, now + std::time::Duration::from_secs(10));
        assert!(is_fresh(&thumb, &source));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_request_produces_nothing() {
        let cancel = Arc::new(AtomicBool::new(true));
        let receiver = request_thumbnail(
            PathBuf::from("/no/such/photo.png"),
            "image/png".to_string(),
            cancel,
        );

        assert_eq!(receiver.recv_blocking().unwrap(), None);
    }
}
