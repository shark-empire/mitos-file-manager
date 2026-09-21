use std::path::PathBuf;

/// How many locations a tab remembers in each direction. Nobody steps back
/// a hundred folders, and an unbounded list is memory a long-lived tab
/// never gets back.
const MAX_ENTRIES: usize = 100;

/// One tab's Back / Forward trail.
///
/// `back_stack` holds the places the tab has been (most recent last);
/// `forward_stack` holds the places Back has stepped away from, so Forward
/// can replay them.
#[derive(Clone, Default)]
pub struct History {
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `path` -- the location being navigated *away from* -- so Back
    /// can return to it. Navigating somewhere new also throws away whatever
    /// Forward could have replayed, like a browser does.
    pub fn push(&mut self, path: PathBuf) {
        self.forward_stack.clear();

        if self.back_stack.last() != Some(&path) {
            self.back_stack.push(path);
        }

        if self.back_stack.len() > MAX_ENTRIES {
            self.back_stack.remove(0);
        }
    }

    /// Take the most recently recorded location off the Back list. This is
    /// the primitive `go_back` is built on.
    pub fn pop(&mut self) -> Option<PathBuf> {
        self.back_stack.pop()
    }

    /// Forget the whole trail, both directions -- for when the places in it
    /// can no longer be trusted to exist (e.g. the drive they were on was
    /// unplugged).
    pub fn clear(&mut self) {
        self.back_stack.clear();
        self.forward_stack.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    /// Step back: returns the location to show, and remembers `current` so
    /// Forward can return to it. Places that have been deleted (or whose
    /// drive has gone) since are skipped rather than landed on.
    pub fn go_back(&mut self, current: &PathBuf) -> Option<PathBuf> {
        let previous = loop {
            let candidate = self.pop()?;

            if candidate.is_dir() {
                break candidate;
            }
        };

        self.forward_stack.push(current.clone());
        Some(previous)
    }

    /// The folders Back would visit, nearest first -- for a "history" menu.
    /// Only folders that still exist are listed, which is exactly the set
    /// `go_back` steps through, so entry N really is N + 1 steps back.
    pub fn back_entries(&self) -> Vec<PathBuf> {
        self.back_stack
            .iter()
            .rev()
            .filter(|path| path.is_dir())
            .cloned()
            .collect()
    }

    /// The folders Forward would visit, nearest first.
    pub fn forward_entries(&self) -> Vec<PathBuf> {
        self.forward_stack
            .iter()
            .rev()
            .filter(|path| path.is_dir())
            .cloned()
            .collect()
    }

    /// Go back `steps` places at once (picking an older entry from the
    /// history menu). Everything passed on the way becomes Forward history,
    /// exactly as if Back had been pressed `steps` times. Lands as far back
    /// as it can if there aren't that many.
    pub fn jump_back(&mut self, steps: usize, current: &PathBuf) -> Option<PathBuf> {
        let mut here = current.clone();
        let mut landed = None;

        for _ in 0..steps.max(1) {
            match self.go_back(&here) {
                Some(previous) => {
                    here = previous.clone();
                    landed = Some(previous);
                }
                None => break,
            }
        }

        landed
    }

    /// `jump_back`, the other way.
    pub fn jump_forward(&mut self, steps: usize, current: &PathBuf) -> Option<PathBuf> {
        let mut here = current.clone();
        let mut landed = None;

        for _ in 0..steps.max(1) {
            match self.go_forward(&here) {
                Some(next) => {
                    here = next.clone();
                    landed = Some(next);
                }
                None => break,
            }
        }

        landed
    }

    /// Step forward again after a `go_back`, skipping places that no longer
    /// exist the same way.
    pub fn go_forward(&mut self, current: &PathBuf) -> Option<PathBuf> {
        let next = loop {
            let candidate = self.forward_stack.pop()?;

            if candidate.is_dir() {
                break candidate;
            }
        };

        self.back_stack.push(current.clone());
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;
    use std::fs;

    /// Three real folders (`history` skips ones that don't exist).
    fn three_dirs(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = scratch_dir(tag);
        let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));

        for dir in [&a, &b, &c] {
            fs::create_dir_all(dir).unwrap();
        }

        (a, b, c)
    }

    #[test]
    fn back_and_forward_retrace_the_trail() {
        let (a, b, c) = three_dirs("history-trail");
        let mut history = History::new();

        // Browse a -> b -> c: each push records the place being left.
        history.push(a.clone());
        history.push(b.clone());

        assert!(history.can_go_back());
        assert!(!history.can_go_forward());

        assert_eq!(history.go_back(&c), Some(b.clone()));
        assert_eq!(history.go_back(&b), Some(a.clone()));
        assert!(!history.can_go_back());

        assert_eq!(history.go_forward(&a), Some(b.clone()));
        assert_eq!(history.go_forward(&b), Some(c));
        assert!(!history.can_go_forward());
    }

    #[test]
    fn navigating_somewhere_new_discards_the_forward_trail() {
        let (a, b, _c) = three_dirs("history-discard");
        let mut history = History::new();

        history.push(a.clone());
        assert_eq!(history.go_back(&b), Some(a.clone()));
        assert!(history.can_go_forward());

        // Now at a; a fresh navigation elsewhere.
        history.push(a);
        assert!(!history.can_go_forward());
    }

    #[test]
    fn folders_that_no_longer_exist_are_skipped() {
        let (a, b, c) = three_dirs("history-vanished");
        let mut history = History::new();

        history.push(a.clone());
        history.push(b.clone());
        fs::remove_dir_all(&b).unwrap();

        assert_eq!(history.go_back(&c), Some(a));
        assert!(!history.can_go_back());
    }

    #[test]
    fn the_history_menu_lists_existing_folders_nearest_first() {
        let (a, b, c) = three_dirs("history-entries");
        let mut history = History::new();

        history.push(a.clone());
        history.push(b.clone());
        assert_eq!(history.back_entries(), vec![b.clone(), a.clone()]);

        // Vanished folders aren't offered.
        fs::remove_dir_all(&a).unwrap();
        assert_eq!(history.back_entries(), vec![b.clone()]);

        assert_eq!(history.go_back(&c), Some(b.clone()));
        assert_eq!(history.forward_entries(), vec![c]);
    }

    #[test]
    fn jumping_back_several_steps_keeps_forward_working() {
        let (a, b, c) = three_dirs("history-jump");
        let mut history = History::new();

        // a -> b -> c, now standing in c.
        history.push(a.clone());
        history.push(b.clone());

        assert_eq!(history.jump_back(2, &c), Some(a.clone()));
        assert!(!history.can_go_back());
        assert_eq!(history.forward_entries(), vec![b.clone(), c.clone()]);

        // And forward the whole way in one go.
        assert_eq!(history.jump_forward(2, &a), Some(c.clone()));

        // Asking for more steps than exist lands as far back as possible.
        assert_eq!(history.jump_back(9, &c), Some(a));
    }

    #[test]
    fn clear_forgets_both_directions() {
        let (a, b, c) = three_dirs("history-clear");
        let mut history = History::new();

        history.push(a);
        history.push(b.clone());
        history.go_back(&c);
        assert!(history.can_go_back() && history.can_go_forward());

        history.clear();
        assert!(!history.can_go_back() && !history.can_go_forward());
    }

    #[test]
    fn repeated_entries_collapse_and_the_trail_is_capped() {
        let mut history = History::new();

        history.push(PathBuf::from("/x"));
        history.push(PathBuf::from("/x"));
        assert_eq!(history.back_stack.len(), 1);

        for i in 0..(MAX_ENTRIES + 20) {
            history.push(PathBuf::from(format!("/x/{i}")));
        }

        assert_eq!(history.back_stack.len(), MAX_ENTRIES);
    }
}
