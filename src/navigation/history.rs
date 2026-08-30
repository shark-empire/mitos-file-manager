use std::path::PathBuf;

#[derive(Clone, Default)]
pub struct History {
    stack: Vec<PathBuf>,
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
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

    pub fn go_back(&mut self, current: &PathBuf) -> Option<PathBuf> {
        if let Some(prev) = self.back_stack.pop() {
            self.forward_stack.push(current.clone());
            Some(prev)
        } else {
            None
        }
    }

    pub fn go_forward(&mut self, current: &PathBuf) -> Option<PathBuf> {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(current.clone());
            Some(next)
        } else {
            None
        }
    }
}
