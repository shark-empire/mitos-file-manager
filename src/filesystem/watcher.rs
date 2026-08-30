use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;

pub struct WatcherManager {
    watcher: Option<RecommendedWatcher>,
    sender: glib::Sender<()>,
}

impl WatcherManager {
    pub fn new(sender: glib::Sender<()>) -> Self {
        Self {
            watcher: None,
            sender,
        }
    }

    pub fn watch(&mut self, path: &Path) {
        // Drop the old watcher before creating a new one
        self.watcher = None;

        let sender = self.sender.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if res.is_ok() {
                    let _ = sender.send(());
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };

        // Watch the directory non-recursively (just the current folder)
        let _ = watcher.watch(path, RecursiveMode::NonRecursive);
        self.watcher = Some(watcher);
    }
}
