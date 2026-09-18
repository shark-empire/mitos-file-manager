# CHANGES

## This session — SaveFiles portal support, and a full re-theme

Two mostly-unrelated pieces of work, both hand-verified only (still no
compiler or display here -- see the note at the end of each).

**`SaveFiles` (the one remaining unimplemented portal method)**

`org.freedesktop.portal.FileChooser`'s `SaveFiles` -- choosing one
destination *folder* for a batch of already-named files -- previously
returned `NotSupported`. It's now implemented:

- Reads `current_folder` (`ay`) and `files` (`aay`) out of the options
  dict via two new helpers, `option_path`/`option_path_list` in
  `xdg_portal.rs`, following the same downcast pattern `option_bool`
  already used for `directory`. Falls back to a single `"Untitled"`
  entry if `files` comes through empty.
- Added a `PortalRequest::SaveFiles` variant (`service.rs`) and a
  matching GTK-thread arm (`main.rs`) that shows the same folder-picker
  `OpenFolder` already uses, then joins the chosen folder with each
  requested filename before replying -- so `begin_request`'s existing
  `uris`-building code in `xdg_portal.rs` didn't need to change at all,
  it just sees one path per input file like it already does for the
  other three methods.
- Also added `save_files` to the custom `org.mitos.FilePicker` interface
  for parity with `open_file`/`save_file`/`open_folder`.

**Re-theme: "Liquid Glass" + sci-fi**

Rewrote `ui/theme.rs`'s CSS for both light and dark mode: translucent
glass panels (toolbar/sidebar/status-bar/popovers) over a deep gradient
window background, continuous rounded corners, a two-tone cyan/violet
accent, and two `@keyframes` animations -- a breathing glow on the
selected row and on the progress bar fill -- kept to just those two
spots on purpose, since animating every row or the whole window
background would cost real CPU/GPU for as long as the window's open,
fighting the "faster, less memory" goal rather than serving it.

Also fixed a real bug found while in here: the old `base_tokens()` put
`mitos_radius` (`8px`), `mitos_font_family`, and `mitos_transition`
through `@define-color`, which only ever registers a *color* -- so none
of the three ever actually resolved, and every `border-radius:
@mitos_radius` in both themes was silently falling back to GTK's
default (square corners) the whole time. New version writes those as
literal values instead of a broken token.

Two things worth knowing before trusting how this looks:
- `cargo check` won't catch a CSS mistake here even once it's clean --
  GTK parses stylesheet content at runtime and silently skips whatever
  it can't parse, no error surfaced either way. Actually launching the
  app is the only real check.
- `gridview`/`columnview`'s item CSS node name is my best recall, not
  verified -- if the grid view's cards don't pick up the hover/selection
  glass treatment, that's the first thing to check (see the comment
  above `SHARED_CSS`).

**Status bar selection count** (from last session's list) turned out to
already exist -- `add_tab` already wires `selection.connect_selection_changed`
to a separate `selection-label` showing "`N` selected · size" next to the
main status label. Missed it last pass because it's set from `add_tab`,
not `refresh_tab`, and stored under a different key (`selection-label`,
not `status-label`). No change needed there.

## Earlier sessions

- Added real multi-window support ("New Window" now opens an actual
  window instead of a placeholder); moved the D-Bus services and config
  load into `main` so they run once per process, not once per window.
- Fixed the first real `cargo check` output: 4 errors (two small
  extraction-logic bugs in `xdg_portal.rs`, a signal-handler-storage bug
  in `grid_view.rs` that needed a new `take_obj_data` util since
  `glib::SignalHandlerId` deliberately isn't `Clone`) and ~70 warnings
  (one unused import; the rest all `gtk::Dialog` deprecated-since-4.10 --
  migrated every dialog in the app to a plain `gtk::Window` scaffold,
  flagged out-of-scope by two earlier sessions before this one actually
  did it).
- Closed the cross-volume trash gap (checks other mounted volumes' own
  trash cans, not just home), added per-item permanent delete and a
  shown deletion date.
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
