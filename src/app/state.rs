use crate::navigation::history::History;
use std::path::PathBuf;

pub struct TabState {
    pub current: PathBuf,
    pub history: History,
    pub show_hidden: bool,
    pub search_query: String,
    /// Bumped by every refresh. A directory listing finishes on a
    /// background thread, and one that comes back after a newer refresh has
    /// started is out of date: it's discarded (or restarted) instead of
    /// painted over the newer state.
    pub load_generation: u64,
    /// A background listing is running for this tab right now.
    pub load_in_flight: bool,
    /// The folder whose contents the tab is currently displaying -- not
    /// necessarily `current` while a new listing is still loading.
    pub loaded_path: PathBuf,
    /// The view is showing search results, not a folder listing. A folder
    /// listing that was already in flight when the results arrived must not
    /// paint over them (an explicit refresh clears this).
    pub showing_results: bool,
}

impl TabState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            current: path,
            history: History::new(),
            show_hidden: crate::config::settings::show_hidden_default(),
            search_query: String::new(),
            load_generation: 0,
            load_in_flight: false,
            loaded_path: PathBuf::new(),
            showing_results: false,
        }
    }
}
