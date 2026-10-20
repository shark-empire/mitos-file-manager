use super::shared::SharedConfig;
use async_channel::Sender;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;

/// Watches `~/.config/mitos/home.conf` and sends a freshly loaded
/// `SharedConfig` whenever its contents actually change.
///
/// Keep the value alive for as long as updates are wanted: dropping it stops
/// the watch.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    pub fn start(sender: Sender<SharedConfig>) -> Option<Self> {
        let config_path = config_path()?;

        // Watch the folder, not the file. Anything that saves by writing a
        // temporary file and renaming it over `home.conf` replaces the
        // file's inode, which would leave a watch on the file itself
        // pointing at nothing -- and a folder watch also notices the file
        // appearing for the first time.
        let folder = config_path.parent()?.to_path_buf();
        let file_name = config_path.file_name()?.to_os_string();

        // So there is something to watch on a first run, before any MITOS
        // app has written a config.
        let _ = std::fs::create_dir_all(&folder);

        // What was last sent (to begin with, what's on disk now), so a save
        // that fires several events -- or changes nothing -- sends one
        // update, or none, instead of refreshing every window each time.
        let mut last_sent = SharedConfig::load();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            let Ok(event) = res else {
                return;
            };

            let touches_config = event
                .paths
                .iter()
                .any(|path| path.file_name() == Some(file_name.as_os_str()));

            if !touches_config || !(event.kind.is_modify() || event.kind.is_create()) {
                return;
            }

            let config = SharedConfig::load();

            if config == last_sent {
                return;
            }

            last_sent = config.clone();
            let _ = sender.send_blocking(config);
        })
        .ok()?;

        watcher.watch(&folder, RecursiveMode::NonRecursive).ok()?;

        Some(Self { _watcher: watcher })
    }
}

fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mitos").join("home.conf"));
        }
    }

    std::env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("mitos")
            .join("home.conf")
    })
}
