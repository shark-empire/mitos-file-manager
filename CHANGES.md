# CHANGES

## This session — first real `cargo check`, fixed everything it found

First pass verified against an actual compiler (GitHub Actions CI log,
not hand-review) rather than best-effort-without-a-compiler like every
session before it. It found exactly 4 errors and 2 warning categories;
all fixed, nothing suppressed or deleted to make a warning go away.

**Errors, all in code from a prior session:**
- `src/portal/xdg_portal.rs`: `option_bool` referenced an undefined `v`
  (leftover from an earlier draft — the bound variable was `value`).
  `save_file`'s `current_name` extraction used `String::try_from(...)`,
  which doesn't exist for `OwnedValue`; switched both spots to
  `.clone().downcast::<T>()`, which the compiler confirms *does* exist
  directly on `OwnedValue` (no `Value::from(...)` wrapper needed — that
  was a guess from before there was a compiler to check it against).
- `src/ui/grid_view.rs`: the video-thumbnail `notify` signal handler was
  computed but never actually stored (`set_obj_data` for it was missing
  entirely — a leftover half-edit), and the cleanup side was reading it
  back as `Rc<SignalHandlerId>` while `.disconnect()` wants a plain
  `SignalHandlerId`. Turns out `glib::SignalHandlerId` deliberately
  doesn't implement `Clone` at all (to prevent double-disconnecting a
  handler) — confirmed from its docs, not guessed — so wrapping it in
  `Rc` to work around that was the wrong fix. Added `take_obj_data` to
  `util.rs` (uses `ObjectExt::steal_data`, which hands back ownership
  directly instead of cloning, so it works for non-`Clone` types), wired
  the missing `set_obj_data` call back in, and used `take_obj_data` on
  the read side instead of `Rc`.

**Warnings:**
- One unused import (`gtk::glib::prelude::*` in `mime/applications.rs`)
  -- `gio::prelude::*` already covers what it was added for. Removed.
- ~70 warnings, all the same root cause: `gtk::Dialog` (and
  `DialogBuilder`/`DialogExt::{add_button,content_area,connect_response}`)
  is deprecated since GTK 4.10 -- flagged as out-of-scope by two earlier
  sessions' CHANGES.md entries, but "fix the warnings" this time meant
  actually doing it rather than deferring a third time. Per GTK's own
  migration notes there's no drop-in replacement widget for a dialog
  with custom content ("just create your own window and add buttons as
  required"), so every dialog in the app (`ui/dialogs.rs`'s five,
  `ui/trash_view.rs`'s one, `main.rs`'s two) is now a plain `gtk::Window`
  built through a small shared scaffold (`dialogs::build_dialog` +
  `dialogs::dialog_button`, now `pub(crate)` so `trash_view.rs`/`main.rs`
  can reuse it too) instead of one Dialog-specific widget each. The two
  that block synchronously on a nested `glib::MainLoop`
  (`choose_conflict_policy`, `confirm_action`) needed a `responded` guard
  added -- closing the window from a button handler also fires
  `connect_close_request`, which would otherwise silently overwrite an
  already-chosen answer with "cancelled."

No compiler here either, so this was hand-verified against the exact
error text/line numbers from the uploaded log rather than a fresh
`cargo check` -- please run one more before merging, though this pass
should be materially more reliable than earlier ones for exactly that
reason: it's reacting to real compiler output instead of guessing at it.

## Earlier sessions

- Closed the cross-volume trash gap: `list`/`restore`/`empty` now also
  check `.Trash-$uid`/`.Trash/$uid` on other mounted volumes, not just
  the home trash; added per-item permanent delete and a shown deletion
  date.
- Added a Settings button to bulk-set mpv/Celluloid and GNOME Text
  Editor as defaults for common video/audio/text MIME types.
- Fixed the status bar's free-space number (was directory-entry size,
  not real `statvfs`), added real async video thumbnails, "Connect to
  Server" for new network mounts, a persisted default view (grid/list)
  setting, a conflict dialog that shows a count, and a real
  `org.freedesktop.portal.FileChooser` implementation alongside the
  existing custom `org.mitos.FilePicker`.
- Wired up the previously-undeclared `desktop` module, backed "Set
  Default App" with real `.desktop`-file writing, made theme changes
  live-reload, verified the archive extractor against zip-slip (already
  safe).
