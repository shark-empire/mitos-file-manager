use crate::navigation::bookmarks::{self, Bookmark};
use crate::operations::undo::UndoableAction;
use crate::operations::PendingOp;
use std::path::PathBuf;

/// How many actions Undo (and, separately, Redo) remembers. Deep enough for
/// a real editing session; unbounded would mean a very long session slowly
/// accumulating a stack of full paths that's never read past the top.
const MAX_UNDO_DEPTH: usize = 50;

pub struct AppContext {
    pub pending: Option<(PendingOp, Vec<PathBuf>)>,
    pub bookmarks: Vec<Bookmark>,
    /// Most recent action last. A new action (anything other than an Undo
    /// or a Redo itself) clears `redo_stack` -- once you've done something
    /// new, "Redo" no longer has a coherent next step.
    pub undo_stack: Vec<UndoableAction>,
    pub redo_stack: Vec<UndoableAction>,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            pending: None,
            bookmarks: bookmarks::load(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Record a completed action as undoable. Call this -- not
    /// `undo_stack.push` directly -- so the depth cap and the
    /// clear-redo-on-new-action rule are never accidentally skipped.
    pub fn push_undo(&mut self, action: UndoableAction) {
        push_capped(&mut self.undo_stack, action);
        self.redo_stack.clear();
    }

    /// Push straight onto the undo stack, leaving the redo stack alone.
    /// Only for what a completed *redo* hands back to reverse itself with
    /// -- `push_undo`'s usual clearing would wipe out any other redo still
    /// waiting, which a redo completing must not do.
    pub fn push_undo_raw(&mut self, action: UndoableAction) {
        push_capped(&mut self.undo_stack, action);
    }

    /// The redo-stack mirror of `push_undo_raw`, for what a completed
    /// *undo* hands back to redo it with.
    pub fn push_redo_raw(&mut self, action: UndoableAction) {
        push_capped(&mut self.redo_stack, action);
    }
}

fn push_capped(stack: &mut Vec<UndoableAction>, action: UndoableAction) {
    stack.push(action);

    if stack.len() > MAX_UNDO_DEPTH {
        stack.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> UndoableAction {
        UndoableAction::CreatedFile {
            path: PathBuf::from(format!("/tmp/{name}")),
        }
    }

    #[test]
    fn push_undo_clears_any_pending_redo() {
        let mut ctx = AppContext::new();

        ctx.push_undo(sample("a"));
        ctx.push_redo_raw(sample("would-be-redo"));
        assert_eq!(ctx.redo_stack.len(), 1);

        // A genuinely new action -- not an undo/redo completing -- means
        // "redo" no longer has a coherent next step.
        ctx.push_undo(sample("b"));
        assert!(ctx.redo_stack.is_empty());
        assert_eq!(ctx.undo_stack.len(), 2);
    }

    #[test]
    fn raw_pushes_do_not_touch_the_other_stack() {
        let mut ctx = AppContext::new();

        ctx.push_undo(sample("a"));
        ctx.push_undo(sample("b"));

        // Simulate completing an Undo: it hands back what Redo should gain,
        // via push_redo_raw -- which must leave the rest of undo_stack
        // (here, empty after two pops) and any other pending redo alone.
        ctx.undo_stack.pop();
        ctx.push_redo_raw(sample("first-undo"));
        ctx.undo_stack.pop();
        ctx.push_redo_raw(sample("second-undo"));

        assert!(ctx.undo_stack.is_empty());
        assert_eq!(ctx.redo_stack.len(), 2);
    }

    #[test]
    fn the_undo_stack_is_capped() {
        let mut ctx = AppContext::new();

        for i in 0..(MAX_UNDO_DEPTH + 10) {
            ctx.push_undo(sample(&i.to_string()));
        }

        assert_eq!(ctx.undo_stack.len(), MAX_UNDO_DEPTH);
    }
}
