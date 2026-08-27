use std::path::PathBuf;

#[derive(Clone, Default)]
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

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}
