# CHANGES

## This session — cross-volume trash, per-item delete, deletion date

`filesystem/trash.rs`'s `list`/`restore`/`empty` only ever looked at the
home trash (`~/.local/share/Trash`). Deleting was already spec-correct —
`operations::trash::delete` (the `trash` crate) puts a file removed from
a USB drive or network share into a `.Trash-$uid`/`.Trash/$uid` can *on
that volume*, per the XDG trash spec — but the trash view couldn't see,
restore, or empty any of those, so they'd sit there invisibly.

- `list`/`empty` now also take a list of (name, mount path) pairs and
  check both spec-defined non-home trash locations on each — see
  `topdir_trash_candidates` for the sticky-bit/symlink checks the spec
  requires before trusting a shared `.Trash` dir.
- `ui/sidebar.rs`'s mount-filtering got pulled out into
  `sidebar::external_mounts()` so the trash view and the sidebar always
  agree on which drives count — `ui/trash_view.rs` feeds that straight
  into `trash::list`/`trash::empty`.
- `TrashItem` gained `location_label` (which trash can an item came from)
  and `deletion_date` (parsed from `.trashinfo`, previously ignored) —
  both now shown per row.
- Added `delete_forever` (permanent single-item delete) and a
  corresponding button, confirmed the same way "Empty Trash" already was.

Same caveat as always: no compiler available here, please `cargo check`
before merging.

## Earlier sessions

- **Recommended defaults**: a Settings button that bulk-sets mpv/Celluloid
  as default for common video/audio MIME types and GNOME Text Editor for
  common text/code ones (`mime/applications.rs::apply_recommended_defaults`).
- **Roadmap-gap pass**: fixed the status bar's free-space number (was
  reading directory-entry size, not `statvfs`), added real video
  thumbnails (`ffmpeg`, async, cached), a "Connect to Server" dialog for
  new `smb://`/`sftp://`/`ftp://` mounts, a persisted default view
  (grid/list) setting, a conflict dialog that says how many files
  conflict, and a real `org.freedesktop.portal.FileChooser` implementation
  (`src/portal/xdg_portal.rs`) alongside the existing custom
  `org.mitos.FilePicker` — the portal code is the least-verified in the
  project (no compiler, no D-Bus session to test against); see the doc
  comment at the top of that file for exactly which parts to check first.
- **CI-warnings pass**: wired up the previously-undeclared `desktop`
  module, backed "Set Default App" with real `.desktop`-file writing,
  made theme changes live-reload, verified the archive extractor against
  zip-slip (already safe). ~40 `GTK4 Dialog` deprecation warnings are
  still untouched (out of scope, `Dialog` → `AlertDialog`/custom windows
  is a bigger migration).
