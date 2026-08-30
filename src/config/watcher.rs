use super::shared::SharedConfig;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    pub fn start(sender: glib::Sender<SharedConfig>) -> Option<Self> {
        let config_path = config_path()?;

        let sender_clone = sender.clone();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() || event.kind.is_create() {
                    let config = SharedConfig::load();
                    let _ = sender_clone.send(config);
                }
            }
        })
        .ok()?;

        watcher
            .watch(&config_path, RecursiveMode::NonRecursive)
            .ok()?;

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
