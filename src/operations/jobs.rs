use crate::operations::{unique_destination, PendingOp};
use gtk::glib;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

#[derive(Clone, Debug)]
pub enum JobMessage {
    Started {
        label: String,
        total: u64,
        bytes: bool,
    },
    Progress {
        label: String,
        processed: u64,
        total: u64,
        bytes: bool,
    },
    Finished {
        result: Result<usize, String>,
    },
}

pub struct JobHandle {
    pub cancel: Arc<AtomicBool>,
}

struct ProgressState {
    sender: glib::Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    label: String,
    total: u64,
    processed: u64,
    bytes: bool,
    last_sent: Instant,
}

impl ProgressState {
    fn new(
        sender: glib::Sender<JobMessage>,
        cancel: Arc<AtomicBool>,
        label: String,
        total: u64,
        bytes: bool,
    ) -> Self {
        Self {
            sender,
            cancel,
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

    fn set(&mut self, value: u64) {
        self.processed = value;
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

pub fn start_paste_job(
    operation: PendingOp,
    sources: Vec<PathBuf>,
    destination: PathBuf,
    sender: glib::Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
    };

    let label = match operation {
        PendingOp::Copy => "Copying".to_string(),
        PendingOp::Move => "Moving".to_string(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = calculate_size(&sources, &cancel).map_err(|err| err.to_string())?;

            let _ = sender.send(JobMessage::Started {
                label: label.clone(),
                total,
                bytes: true,
            });

            let mut state = ProgressState::new(
                sender.clone(),
                cancel.clone(),
                label.clone(),
                total,
                true,
            );

            let mut completed = 0;

            for source in &sources {
                check_cancel(&cancel).map_err(|err| err.to_string())?;

                if !source.exists() {
                    continue;
                }

                if destination.starts_with(source) || source.as_path() == destination.as_path() {
                    continue;
                }

                if source.parent() == Some(destination.as_path()) {
                    continue;
                }

                let file_name = source.file_name().unwrap_or_default().to_os_string();
                let target = unique_destination(&destination.join(file_name));

                match operation {
                    PendingOp::Copy => copy_path_with_progress(source, &target, &mut state),
                    PendingOp::Move => move_path_with_progress(source, &target, &mut state),
                }
                .map_err(|err| err.to_string())?;

                completed += 1;
            }

            state.set(total);

            Ok(completed)
        })();

        let _ = sender.send(JobMessage::Finished { result });
    });

    handle
}

pub fn start_trash_job(
    paths: Vec<PathBuf>,
    sender: glib::Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = paths.len() as u64;

            let _ = sender.send(JobMessage::Started {
                label: "Moving to trash".to_string(),
                total,
                bytes: false,
            });

            let mut state = ProgressState::new(
                sender.clone(),
                cancel.clone(),
                "Moving to trash".to_string(),
                total,
                false,
            );

            let mut completed = 0;

            for (index, path) in paths.iter().enumerate() {
                check_cancel(&cancel).map_err(|err| err.to_string())?;

                crate::operations::trash::delete(path).map_err(|err| err.to_string())?;

                completed += 1;
                state.set((index + 1) as u64);
            }

            Ok(completed)
        })();

        let _ = sender.send(JobMessage::Finished { result });
    });

    handle
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Cancelled",
        ))
    } else {
        Ok(())
    }
}

fn calculate_size(paths: &[PathBuf], cancel: &AtomicBool) -> io::Result<u64> {
    let mut total = 0;

    for path in paths {
        check_cancel(cancel)?;
        total += path_size(path, cancel)?;
    }

    Ok(total)
}

fn path_size(path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    check_cancel(cancel)?;

    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        let mut total = 0;

        for entry in fs::read_dir(path)? {
            check_cancel(cancel)?;

            let entry = entry?;
            total += path_size(&entry.path(), cancel)?;
        }

        Ok(total)
    } else {
        Ok(metadata.len())
    }
}

fn copy_path_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel(&state.cancel)?;

    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir_with_progress(source, destination, state)
    } else if metadata.file_type().is_symlink() {
        copy_symlink(source, destination)
    } else {
        copy_file_with_progress(source, destination, state)
    }
}

fn copy_dir_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel(&state.cancel)?;

    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        check_cancel(&state.cancel)?;

        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_with_progress(&entry.path(), &target, state)?;
        } else if file_type.is_symlink() {
            copy_symlink(&entry.path(), &target)?;
        } else {
            copy_file_with_progress(&entry.path(), &target, state)?;
        }
    }

    Ok(())
}

fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    let target = fs::read_link(source)?;

    #[cfg(unix)]
    symlink_unix(target, destination)?;

    #[cfg(not(unix))]
    {
        let _ = target;
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Symlink copying is not supported on this platform",
        ));
    }

    Ok(())
}

fn copy_file_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    let result = copy_file_chunks(source, destination, state);

    if result.is_err() {
        let _ = fs::remove_file(destination);
    }

    result
}

fn copy_file_chunks(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel(&state.cancel)?;

    let mut reader = fs::File::open(source)?;
    let mut writer = fs::File::create(destination)?;

    let mut buffer = [0u8; 64 * 1024];

    loop {
        check_cancel(&state.cancel)?;

        let read = reader.read(&mut buffer)?;

        if read == 0 {
            break;
        }

        writer.write_all(&buffer[..read])?;
        state.add(read as u64);
    }

    writer.flush()?;

    if let Ok(metadata) = fs::metadata(source) {
        let _ = fs::set_permissions(destination, metadata.permissions());
    }

    Ok(())
}

fn move_path_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel(&state.cancel)?;

    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_path_with_progress(source, destination, state)?;
                remove_all_with_progress(source, state)?;
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn remove_all_with_progress(path: &Path, state: &mut ProgressState) -> io::Result<()> {
    check_cancel(&state.cancel)?;

    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            check_cancel(&state.cancel)?;

            let entry = entry?;
            remove_all_with_progress(&entry.path(), state)?;
        }

        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}
