use crate::operations::jobs::{JobHandle, JobMessage};
use crate::operations::unique_destination;
use flate2::read::GzDecoder;
use gtk::glib;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tar::Archive;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

pub fn is_supported_archive(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    name.ends_with(".zip")
        || name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
}

pub fn default_archive_path(destination_dir: &Path) -> PathBuf {
    unique_destination(&destination_dir.join("Archive.zip"))
}

pub fn default_extract_dir(destination_dir: &Path, archive_path: &Path) -> PathBuf {
    let name = archive_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Extracted".to_string());

    let folder_name = archive_folder_name(&name);

    unique_destination(&destination_dir.join(folder_name))
}

fn archive_folder_name(name: &str) -> String {
    if let Some(stripped) = name.strip_suffix(".tar.gz") {
        return stripped.to_string();
    }

    if let Some(stripped) = name.strip_suffix(".tgz") {
        return stripped.to_string();
    }

    if let Some(stripped) = name.strip_suffix(".zip") {
        return stripped.to_string();
    }

    if let Some(stripped) = name.strip_suffix(".tar") {
        return stripped.to_string();
    }

    "Extracted".to_string()
}

pub fn start_compress_zip_job(
    sources: Vec<PathBuf>,
    archive_path: PathBuf,
    sender: glib::Sender<JobMessage>,
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

            let _ = sender.send(JobMessage::Started {
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

        let _ = sender.send(JobMessage::Finished { result });
    });

    handle
}

pub fn start_extract_job(
    archive_path: PathBuf,
    destination_dir: PathBuf,
    sender: glib::Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
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
            } else {
                Err("Unsupported archive type".to_string())
            }
        })();

        let _ = sender.send(JobMessage::Finished { result });
    });

    handle
}

struct ArchiveProgress {
    sender: glib::Sender<JobMessage>,
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
        sender: glib::Sender<JobMessage>,
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
            let _ = self.sender.send(JobMessage::Progress {
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
    sender: glib::Sender<JobMessage>,
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

    let _ = sender.send(JobMessage::Started {
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
                let _ = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode));
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
    sender: glib::Sender<JobMessage>,
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
    sender: glib::Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let file = fs::File::open(archive_path).map_err(|err| err.to_string())?;
    extract_tar_reader(file, destination_dir, sender, cancel, pause)
}

fn extract_tar_reader<R: Read>(
    reader: R,
    destination_dir: &Path,
    sender: glib::Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    let _ = sender.send(JobMessage::Started {
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
