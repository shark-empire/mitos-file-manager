use crate::operations::jobs::{JobHandle, JobMessage};
use crate::operations::unique_destination;
use async_channel::Sender;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tar::{Archive, Builder};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// Formats this app reads itself, with no help from anything installed.
const NATIVE_SUFFIXES: &[&str] = &[".zip", ".tar", ".tar.gz", ".tgz"];

/// Formats handed to an external extractor (`bsdtar` or `7z`), if one is
/// installed. Reading these natively would mean more dependencies for
/// formats most people rarely open.
const EXTERNAL_SUFFIXES: &[&str] = &[
    ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst", ".7z", ".rar",
];

fn lowercase_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Can "Extract Here" do something with this file?
pub fn is_supported_archive(path: &Path) -> bool {
    let name = lowercase_name(path);

    NATIVE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
        || (EXTERNAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
            && external_extractor().is_some())
}

/// A name for a new archive of `sources` inside `destination_dir`: after the
/// item itself when there's just one ("Photos.zip"), "Archive.zip"
/// otherwise -- never colliding with an existing file. `extension` has no
/// leading dot ("zip", "tar.gz").
pub fn default_archive_path(
    destination_dir: &Path,
    sources: &[PathBuf],
    extension: &str,
) -> PathBuf {
    let base = match sources {
        [only] => only
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Archive".to_string()),
        _ => "Archive".to_string(),
    };

    unique_destination(&destination_dir.join(format!("{base}.{extension}")))
}

pub fn default_extract_dir(destination_dir: &Path, archive_path: &Path) -> PathBuf {
    let name = archive_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Extracted".to_string());

    let folder_name = archive_folder_name(&name);

    unique_destination(&destination_dir.join(folder_name))
}

/// "photos.tar.gz" -> "photos": the name of the folder to extract into.
fn archive_folder_name(name: &str) -> String {
    let lower = name.to_lowercase();

    for suffix in NATIVE_SUFFIXES.iter().chain(EXTERNAL_SUFFIXES) {
        // The suffixes are ASCII and `to_lowercase` keeps ASCII lengths, so
        // the cut point is the same in both spellings.
        if lower.ends_with(suffix) && name.len() > suffix.len() && lower.len() == name.len() {
            return name[..name.len() - suffix.len()].to_string();
        }
    }

    "Extracted".to_string()
}

pub fn start_compress_zip_job(
    sources: Vec<PathBuf>,
    archive_path: PathBuf,
    sender: Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = calculate_total_size(&sources, &cancel).map_err(|err| err.to_string())?;

            let _ = sender.send_blocking(JobMessage::Started {
                label: "Compressing".to_string(),
                total,
                bytes: true,
            });

            let mut progress = ArchiveProgress::new(
                sender.clone(),
                cancel.clone(),
                pause.clone(),
                "Compressing".to_string(),
                total,
                true,
            );

            if let Some(parent) = archive_path.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }

            let file = fs::File::create(&archive_path).map_err(|err| err.to_string())?;
            let mut zip = ZipWriter::new(file);

            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o644);

            let mut completed = 0usize;

            for source in &sources {
                check_cancel_and_pause(&cancel, &pause).map_err(|err| err.to_string())?;

                if !source.exists() {
                    continue;
                }

                let base = source.parent().unwrap_or_else(|| Path::new(""));
                add_path_to_zip(&mut zip, source, base, options, &mut progress)
                    .map_err(|err| err.to_string())?;

                completed += 1;
            }

            zip.finish().map_err(|err| err.to_string())?;

            progress.set(total);

            Ok(completed)
        })();

        // A cancelled or failed compression must not leave a truncated,
        // corrupt .zip behind that looks like a real archive.
        if result.is_err() {
            let _ = fs::remove_file(&archive_path);
        }

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

/// Compress `sources` into a gzip-compressed tar archive. Symlinks are
/// stored as links (not followed), and permissions and timestamps travel
/// with the files -- which plain ZIPs handle poorly.
pub fn start_compress_tar_gz_job(
    sources: Vec<PathBuf>,
    archive_path: PathBuf,
    sender: Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = calculate_total_size(&sources, &cancel).map_err(|err| err.to_string())?;

            let _ = sender.send_blocking(JobMessage::Started {
                label: "Compressing".to_string(),
                total,
                bytes: true,
            });

            let mut progress = ArchiveProgress::new(
                sender.clone(),
                cancel.clone(),
                pause.clone(),
                "Compressing".to_string(),
                total,
                true,
            );

            if let Some(parent) = archive_path.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }

            let file = fs::File::create(&archive_path).map_err(|err| err.to_string())?;
            let mut builder = Builder::new(GzEncoder::new(file, Compression::default()));
            builder.follow_symlinks(false);

            let mut completed = 0usize;

            for source in &sources {
                check_cancel_and_pause(&cancel, &pause).map_err(|err| err.to_string())?;

                if fs::symlink_metadata(source).is_err() {
                    continue;
                }

                let Some(name) = source.file_name() else {
                    continue;
                };

                add_path_to_tar(&mut builder, source, Path::new(name), &mut progress)
                    .map_err(|err| err.to_string())?;

                completed += 1;
            }

            // `into_inner` writes the tar trailer; `finish` flushes gzip.
            builder
                .into_inner()
                .and_then(|encoder| encoder.finish())
                .map_err(|err| err.to_string())?;

            progress.set(total);

            Ok(completed)
        })();

        if result.is_err() {
            let _ = fs::remove_file(&archive_path);
        }

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

fn add_path_to_tar<W: Write>(
    builder: &mut Builder<W>,
    source: &Path,
    name_in_archive: &Path,
    progress: &mut ArchiveProgress,
) -> io::Result<()> {
    check_cancel_and_pause(&progress.cancel, &progress.pause)?;

    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        builder.append_dir(name_in_archive, source)?;

        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let child_name = name_in_archive.join(entry.file_name());

            add_path_to_tar(builder, &entry.path(), &child_name, progress)?;
        }
    } else {
        // Symlinks are stored as symlinks (`follow_symlinks(false)`); regular
        // files are streamed from disk, not read into memory.
        builder.append_path_with_name(source, name_in_archive)?;
        progress.add(metadata.len());
    }

    Ok(())
}

pub fn start_extract_job(
    archive_path: PathBuf,
    destination_dir: PathBuf,
    sender: Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        // Whether `destination_dir` is ours to clean up if this fails: only
        // if it didn't exist before.
        let created_destination = !destination_dir.exists();

        let result = (|| -> Result<usize, String> {
            fs::create_dir_all(&destination_dir).map_err(|err| err.to_string())?;

            let name = archive_path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if name.ends_with(".zip") {
                extract_zip(
                    &archive_path,
                    &destination_dir,
                    sender.clone(),
                    cancel,
                    pause,
                )
            } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                extract_tar_gz(
                    &archive_path,
                    &destination_dir,
                    sender.clone(),
                    cancel,
                    pause,
                )
            } else if name.ends_with(".tar") {
                extract_tar(
                    &archive_path,
                    &destination_dir,
                    sender.clone(),
                    cancel,
                    pause,
                )
            } else if EXTERNAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
                match external_extractor() {
                    Some(tool) => extract_external(
                        &tool,
                        &archive_path,
                        &destination_dir,
                        sender.clone(),
                        &cancel,
                    ),
                    None => Err(
                        "Extracting this kind of archive needs bsdtar or 7z, and neither is installed"
                            .to_string(),
                    ),
                }
            } else {
                Err("Unsupported archive type".to_string())
            }
        })();

        // An extraction that failed part-way must not leave a folder full of
        // half-written files. (Only ever removes a folder this job created;
        // a cancelled extraction counts as failed.)
        if result.is_err() && created_destination {
            let _ = fs::remove_dir_all(&destination_dir);
        }

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

struct ArchiveProgress {
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    label: String,
    total: u64,
    processed: u64,
    bytes: bool,
    last_sent: Instant,
}

impl ArchiveProgress {
    fn new(
        sender: Sender<JobMessage>,
        cancel: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
        label: String,
        total: u64,
        bytes: bool,
    ) -> Self {
        Self {
            sender,
            cancel,
            pause,
            label,
            total,
            processed: 0,
            bytes,
            last_sent: Instant::now(),
        }
    }

    fn add(&mut self, amount: u64) {
        self.processed += amount;
        self.maybe_send();
    }

    fn set(&mut self, amount: u64) {
        self.processed = amount;
        self.maybe_send();
    }

    fn maybe_send(&mut self) {
        if self.last_sent.elapsed().as_millis() >= 80 {
            let _ = self.sender.send_blocking(JobMessage::Progress {
                label: self.label.clone(),
                processed: self.processed,
                total: self.total,
                bytes: self.bytes,
            });

            self.last_sent = Instant::now();
        }
    }
}

fn check_cancel_and_pause(cancel: &AtomicBool, pause: &AtomicBool) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }

        if !pause.load(Ordering::Relaxed) {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn calculate_total_size(paths: &[PathBuf], cancel: &AtomicBool) -> io::Result<u64> {
    let mut total = 0;

    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }

        total += path_size(path, cancel)?;
    }

    Ok(total)
}

fn path_size(path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
    }

    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        let mut total = 0;

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            total += path_size(&entry.path(), cancel)?;
        }

        Ok(total)
    } else {
        Ok(metadata.len())
    }
}

fn add_path_to_zip(
    zip: &mut ZipWriter<fs::File>,
    source: &Path,
    base: &Path,
    options: SimpleFileOptions,
    progress: &mut ArchiveProgress,
) -> io::Result<()> {
    check_cancel_and_pause(&progress.cancel, &progress.pause)?;

    let metadata = fs::symlink_metadata(source)?;

    let relative = source.strip_prefix(base).unwrap_or(source);
    let zip_name = zip_path(relative);

    if metadata.is_dir() {
        if !zip_name.is_empty() {
            let dir_name = format!("{}/", zip_name.trim_end_matches('/'));
            zip.add_directory(dir_name, options)?;
        }

        for entry in fs::read_dir(source)? {
            let entry = entry?;
            add_path_to_zip(zip, &entry.path(), base, options, progress)?;
        }
    } else if metadata.file_type().is_symlink() {
        // For now, skip symlinks in zip archives.
        // This avoids accidentally storing broken platform-specific links.
    } else {
        zip.start_file(zip_name, options)?;

        let mut file = fs::File::open(source)?;
        let mut buffer = [0u8; 64 * 1024];

        loop {
            check_cancel_and_pause(&progress.cancel, &progress.pause)?;

            let read = file.read(&mut buffer)?;

            if read == 0 {
                break;
            }

            zip.write_all(&buffer[..read])?;
            progress.add(read as u64);
        }
    }

    Ok(())
}

fn zip_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn extract_zip(
    archive_path: &Path,
    destination_dir: &Path,
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let file = fs::File::open(archive_path).map_err(|err| err.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|err| err.to_string())?;

    let mut total = 0u64;

    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|err| err.to_string())?;
        total += file.size();
    }

    crate::filesystem::metadata::ensure_free_space(destination_dir, total)?;

    let _ = sender.send_blocking(JobMessage::Started {
        label: "Extracting".to_string(),
        total,
        bytes: true,
    });

    let mut progress = ArchiveProgress::new(
        sender,
        cancel.clone(),
        pause.clone(),
        "Extracting".to_string(),
        total,
        true,
    );

    let mut completed = 0usize;

    for i in 0..archive.len() {
        check_cancel_and_pause(&cancel, &pause).map_err(|err| err.to_string())?;

        let mut file = archive.by_index(i).map_err(|err| err.to_string())?;

        let Some(enclosed_name) = file.enclosed_name().map(|path| path.to_owned()) else {
            continue;
        };

        let mut outpath = destination_dir.join(enclosed_name);

        if file.is_dir() {
            fs::create_dir_all(&outpath).map_err(|err| err.to_string())?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }

        if outpath.exists() {
            outpath = unique_destination(&outpath);
        }

        let mut outfile = fs::File::create(&outpath).map_err(|err| err.to_string())?;
        let mut buffer = [0u8; 64 * 1024];

        loop {
            check_cancel_and_pause(&cancel, &pause).map_err(|err| err.to_string())?;

            let read = file.read(&mut buffer).map_err(|err| err.to_string())?;

            if read == 0 {
                break;
            }

            outfile
                .write_all(&buffer[..read])
                .map_err(|err| err.to_string())?;

            progress.add(read as u64);
        }

        #[cfg(unix)]
        {
            if let Some(mode) = file.unix_mode() {
                use std::os::unix::fs::PermissionsExt;

                // Only the rwx bits: an archive must not be able to hand out
                // files with the setuid / setgid / sticky bit already set.
                let _ = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode & 0o777));
            }
        }

        completed += 1;
    }

    progress.set(total);

    Ok(completed)
}

fn extract_tar_gz(
    archive_path: &Path,
    destination_dir: &Path,
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let file = fs::File::open(archive_path).map_err(|err| err.to_string())?;
    let decoder = GzDecoder::new(file);
    extract_tar_reader(decoder, destination_dir, sender, cancel, pause)
}

fn extract_tar(
    archive_path: &Path,
    destination_dir: &Path,
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let file = fs::File::open(archive_path).map_err(|err| err.to_string())?;
    extract_tar_reader(file, destination_dir, sender, cancel, pause)
}

fn extract_tar_reader<R: Read>(
    reader: R,
    destination_dir: &Path,
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let _ = sender.send_blocking(JobMessage::Started {
        label: "Extracting".to_string(),
        total: 0,
        bytes: false,
    });

    let mut progress = ArchiveProgress::new(
        sender,
        cancel.clone(),
        pause.clone(),
        "Extracting".to_string(),
        0,
        false,
    );

    let mut archive = Archive::new(reader);
    let entries = archive.entries().map_err(|err| err.to_string())?;

    let mut completed = 0usize;

    for entry in entries {
        check_cancel_and_pause(&cancel, &pause).map_err(|err| err.to_string())?;

        let mut entry = entry.map_err(|err| err.to_string())?;

        // unpack_in prevents path traversal outside destination_dir.
        entry
            .unpack_in(destination_dir)
            .map_err(|err| err.to_string())?;

        completed += 1;
        progress.add(1);
    }

    Ok(completed)
}

// ---------------------------------------------------------------------------
// External extractors
// ---------------------------------------------------------------------------

enum ExternalTool {
    /// libarchive's `bsdtar`: reads tar.xz / tar.bz2 / tar.zst / 7z / rar ...
    Bsdtar(PathBuf),
    /// 7-Zip (`7zz`, `7z` or `7za`).
    SevenZip(PathBuf),
}

impl ExternalTool {
    fn name(&self) -> &'static str {
        match self {
            ExternalTool::Bsdtar(_) => "bsdtar",
            ExternalTool::SevenZip(_) => "7z",
        }
    }

    fn command(&self, archive_path: &Path, destination_dir: &Path) -> std::process::Command {
        match self {
            ExternalTool::Bsdtar(program) => {
                // bsdtar refuses absolute paths and ".." components by default.
                let mut command = std::process::Command::new(program);
                command
                    .arg("-x")
                    .arg("-f")
                    .arg(archive_path)
                    .arg("-C")
                    .arg(destination_dir);
                command
            }
            ExternalTool::SevenZip(program) => {
                let mut command = std::process::Command::new(program);
                command
                    .arg("x")
                    .arg("-y")
                    .arg(format!("-o{}", destination_dir.display()))
                    .arg(archive_path);
                command
            }
        }
    }
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;

    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn external_extractor() -> Option<ExternalTool> {
    if let Some(path) = find_in_path("bsdtar") {
        return Some(ExternalTool::Bsdtar(path));
    }

    ["7zz", "7z", "7za"]
        .iter()
        .find_map(|name| find_in_path(name))
        .map(ExternalTool::SevenZip)
}

fn extract_external(
    tool: &ExternalTool,
    archive_path: &Path,
    destination_dir: &Path,
    sender: Sender<JobMessage>,
    cancel: &AtomicBool,
) -> Result<usize, String> {
    let _ = sender.send_blocking(JobMessage::Started {
        label: "Extracting".to_string(),
        total: 0,
        bytes: false,
    });

    let mut command = tool.command(archive_path, destination_dir);

    // Nothing to read from us, and nothing worth capturing: an unread pipe
    // that fills up would stall the tool.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|err| format!("Couldn't start {}: {err}", tool.name()))?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cancelled".to_string());
        }

        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(1),
            Ok(Some(status)) => {
                return Err(format!(
                    "{} could not extract this archive ({status})",
                    tool.name()
                ))
            }
            Ok(None) => {
                // Unknown total: keep the progress bar pulsing.
                let _ = sender.send_blocking(JobMessage::Progress {
                    label: "Extracting".to_string(),
                    processed: 0,
                    total: 0,
                    bytes: false,
                });

                thread::sleep(Duration::from_millis(150));
            }
            Err(err) => return Err(err.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Listing (for the preview panel)
// ---------------------------------------------------------------------------

/// The names inside an archive, for a preview -- without extracting it.
pub struct ArchiveListing {
    pub entries: Vec<String>,
    /// There were more than `limit` entries; `entries` is only the start.
    pub truncated: bool,
}

/// List up to `limit` entries of a ZIP or (gzip-compressed) tar archive.
/// Formats that need an external tool aren't listed.
pub fn list_entries(path: &Path, limit: usize) -> Result<ArchiveListing, String> {
    let name = lowercase_name(path);
    let file = fs::File::open(path).map_err(|err| err.to_string())?;

    if name.ends_with(".zip") {
        let archive = ZipArchive::new(file).map_err(|err| err.to_string())?;

        let mut entries: Vec<String> = archive.file_names().take(limit + 1).map(String::from).collect();
        let truncated = entries.len() > limit;
        entries.truncate(limit);

        return Ok(ArchiveListing { entries, truncated });
    }

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        return list_tar(GzDecoder::new(file), limit);
    }

    if name.ends_with(".tar") {
        return list_tar(file, limit);
    }

    Err("This kind of archive can't be previewed".to_string())
}

fn list_tar<R: Read>(reader: R, limit: usize) -> Result<ArchiveListing, String> {
    let mut archive = Archive::new(reader);
    let mut entries = Vec::new();
    let mut truncated = false;

    for entry in archive.entries().map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;

        if entries.len() >= limit {
            truncated = true;
            break;
        }

        let path = entry.path().map_err(|err| err.to_string())?;
        entries.push(path.to_string_lossy().to_string());
    }

    Ok(ArchiveListing { entries, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    /// Drain a job's messages and return its final result.
    fn finish(receiver: async_channel::Receiver<JobMessage>) -> Result<usize, String> {
        loop {
            match receiver.recv_blocking() {
                Ok(JobMessage::Finished { result }) => return result,
                Ok(_) => continue,
                Err(_) => return Err("job ended without a result".to_string()),
            }
        }
    }

    /// root/docs/{a.txt, sub/b.txt, link-to-a -> a.txt}; returns `docs`.
    fn sample_tree(tag: &str) -> PathBuf {
        let root = scratch_dir(tag);
        let docs = root.join("docs");

        fs::create_dir_all(docs.join("sub")).unwrap();
        fs::write(docs.join("a.txt"), "alpha").unwrap();
        fs::write(docs.join("sub").join("b.txt"), "beta").unwrap();
        std::os::unix::fs::symlink("a.txt", docs.join("link-to-a")).unwrap();

        docs
    }

    #[test]
    fn tar_gz_round_trip_keeps_files_and_symlinks() {
        let docs = sample_tree("archive-targz");
        let root = docs.parent().unwrap().to_path_buf();

        let archive = default_archive_path(&root, &[docs.clone()], "tar.gz");
        assert_eq!(archive, root.join("docs.tar.gz"));

        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_compress_tar_gz_job(vec![docs.clone()], archive.clone(), sender);
        assert_eq!(finish(receiver), Ok(1));
        assert!(archive.exists());

        let out = root.join("out");
        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_extract_job(archive.clone(), out.clone(), sender);
        assert!(finish(receiver).is_ok());

        assert_eq!(fs::read_to_string(out.join("docs/a.txt")).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(out.join("docs/sub/b.txt")).unwrap(), "beta");
        assert_eq!(
            fs::read_link(out.join("docs/link-to-a")).unwrap(),
            PathBuf::from("a.txt")
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zip_round_trip_and_listing() {
        let docs = sample_tree("archive-zip");
        let root = docs.parent().unwrap().to_path_buf();

        let archive = default_archive_path(&root, &[docs.clone()], "zip");
        assert_eq!(archive, root.join("docs.zip"));

        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_compress_zip_job(vec![docs.clone()], archive.clone(), sender);
        assert!(finish(receiver).is_ok());

        let listing = list_entries(&archive, 100).unwrap();
        assert!(listing.entries.iter().any(|name| name == "docs/a.txt"));
        assert!(!listing.truncated);

        let short = list_entries(&archive, 1).unwrap();
        assert_eq!(short.entries.len(), 1);
        assert!(short.truncated);

        let out = root.join("out");
        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_extract_job(archive.clone(), out.clone(), sender);
        assert!(finish(receiver).is_ok());

        assert_eq!(fs::read_to_string(out.join("docs/a.txt")).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(out.join("docs/sub/b.txt")).unwrap(), "beta");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tar_gz_listing_shows_the_files_inside() {
        let docs = sample_tree("archive-tar-listing");
        let root = docs.parent().unwrap().to_path_buf();
        let archive = root.join("docs.tar.gz");

        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_compress_tar_gz_job(vec![docs], archive.clone(), sender);
        assert!(finish(receiver).is_ok());

        let listing = list_entries(&archive, 100).unwrap();
        assert!(listing.entries.iter().any(|name| name.ends_with("a.txt")));
        assert!(list_entries(&root.join("nope.rar"), 10).is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn zip_entries_that_climb_out_of_the_folder_are_skipped() {
        let root = scratch_dir("archive-zipslip");
        let evil = root.join("evil.zip");

        {
            let mut zip = ZipWriter::new(fs::File::create(&evil).unwrap());
            let options = SimpleFileOptions::default();

            // If this version of the zip crate refuses to even write such a
            // name there's nothing to test.
            if zip.start_file("../escaped.txt", options).is_err() {
                return;
            }

            zip.write_all(b"gotcha").unwrap();
            zip.start_file("safe/inside.txt", options).unwrap();
            zip.write_all(b"fine").unwrap();
            zip.finish().unwrap();
        }

        let dest = root.join("dest");
        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_extract_job(evil, dest.clone(), sender);
        assert!(finish(receiver).is_ok());

        assert!(!root.join("escaped.txt").exists());
        assert_eq!(fs::read_to_string(dest.join("safe/inside.txt")).unwrap(), "fine");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_extraction_removes_the_folder_it_created() {
        let root = scratch_dir("archive-cleanup");
        let not_really_a_zip = root.join("broken.zip");
        fs::write(&not_really_a_zip, "this is not a zip file").unwrap();

        let dest = root.join("dest");
        let (sender, receiver) = async_channel::unbounded();
        let _handle = start_extract_job(not_really_a_zip, dest.clone(), sender);

        assert!(finish(receiver).is_err());
        assert!(!dest.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_names_are_derived_sensibly() {
        let dir = Path::new("/nonexistent-folder");

        assert_eq!(
            default_archive_path(dir, &[PathBuf::from("/x/Photos")], "zip"),
            dir.join("Photos.zip")
        );
        assert_eq!(
            default_archive_path(dir, &[PathBuf::from("/x/a"), PathBuf::from("/x/b")], "tar.gz"),
            dir.join("Archive.tar.gz")
        );

        assert_eq!(archive_folder_name("Photos.TAR.GZ"), "Photos");
        assert_eq!(archive_folder_name("backup.tgz"), "backup");
        assert_eq!(archive_folder_name("old.tar.xz"), "old");
        assert_eq!(archive_folder_name("plain"), "Extracted");
        assert_eq!(archive_folder_name(".zip"), "Extracted");

        assert!(is_supported_archive(Path::new("a.ZIP")));
        assert!(is_supported_archive(Path::new("a.tar.gz")));
        assert!(!is_supported_archive(Path::new("a.txt")));
    }
}
