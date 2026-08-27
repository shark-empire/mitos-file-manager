use crate::navigation::bookmarks::{self, Bookmark};
use crate::operations::PendingOp;
use std::path::PathBuf;

pub struct AppContext {
    pub pending: Option<(PendingOp, Vec<PathBuf>)>,
    pub bookmarks: Vec<Bookmark>,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            pending: None,
            bookmarks: bookmarks::load(),
        }
    }
}
