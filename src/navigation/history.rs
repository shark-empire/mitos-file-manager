use std::path::PathBuf;

#[derive(Default)]
pub struct History {
    stack: Vec<PathBuf>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, path: PathBuf) {
        self.stack.push(path);
    }

    pub fn pop(&mut self) -> Option<PathBuf> {
        self.stack.pop()
    }
}
