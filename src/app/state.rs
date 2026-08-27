use crate::filesystem::directory::Item;
use crate::navigation::bookmarks::{self, Bookmark};
use crate::navigation::history::History;
use crate::operations::PendingOp;
use std::path::PathBuf;

pub struct AppState {
    pub current: PathBuf,
    pub history: History,
    pub pending: Option<(PendingOp, Vec<PathBuf>)>,
    pub show_hidden: bool,
    pub items: Vec<Item>,
    pub bookmarks: Vec<Bookmark>,
}

impl AppState {
    pub fn new(home: PathBuf) -> Self {
        Self {
            current: home,
            history: History::new(),
            pending: None,
            show_hidden: false,
            items: Vec::new(),
            bookmarks: bookmarks::load(),
        }
    }
}
