use crate::filesystem::directory::Item;
use crate::navigation::history::History;
use std::path::PathBuf;

pub struct TabState {
    pub current: PathBuf,
    pub history: History,
    pub show_hidden: bool,
    pub items: Vec<Item>,
    pub search_query: String,
}

impl TabState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            current: path,
            history: History::new(),
            show_hidden: false,
            items: Vec::new(),
            search_query: String::new(),
        }
    }
}
