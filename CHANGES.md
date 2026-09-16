# CHANGES

## This session — recommended default apps

Added a "Set Recommended Defaults" button to Settings
(`mime/applications.rs::apply_recommended_defaults`, wired into
`ui/settings.rs`): sets mpv (falling back to Celluloid if mpv isn't
installed) as the default for a curated list of common video and audio
MIME types, and GNOME Text Editor for a curated list of text/code MIME
types. Reuses the exact same `set_default_app`/`gio::AppInfo` mechanism
the per-file Properties > Open With tab already used — this is just that,
applied in bulk to a hardcoded MIME list instead of one file at a time.

Skips (doesn't error on) any category whose app isn't installed — the
status label after clicking says exactly which categories were set and
which were skipped. `gio::DesktopAppInfo::new("mpv.desktop")` returning
`None` means mpv isn't on the system, not a bug.

The MIME lists are curated, not exhaustive — common containers/codecs for
video and audio, `text/plain` + `text/markdown` + a handful of common
source-file types for text. Add more to `RECOMMENDED_DEFAULTS` in
`mime/applications.rs` if a format you use isn't covered.

Same caveat as every session so far: no compiler available, please
`cargo check` before merging.

## Previous session — closing out the Phase 1-5 roadmap gaps

Like the previous pass, no network and no Rust toolchain were available
while making these changes, so **please run `cargo check` / `cargo build`
/ `cargo clippy` before merging** — especially `src/portal/xdg_portal.rs`,
flagged in detail below.

Audited the whole roadmap first rather than assuming what was missing —
most of Phase 1-4 turned out to already be implemented (keyboard
shortcuts, the clickable breadcrumb bar, the inotify watcher, trash,
archives, batch rename, image thumbnails, the GVfs sidebar with live
mount/unmount). Fixed what was actually broken or missing:

- **Free space was wrong.** The status bar's "free space" was reading
  `symlink_metadata(path).len()` — the directory entry's own size, not
  disk space. Now uses `statvfs(3)` via a small `libc` FFI call
  (`filesystem/metadata.rs::free_space_string`), matching what `df`
  reports (`f_bavail`, not `f_bfree` — excludes the root-reserved
  margin).
- **Video thumbnails.** Images already had real thumbnails; videos only
  showed one if some *other* tool had already populated the freedesktop
  thumbnail cache. `mime/thumbnail.rs` now generates one itself via a
  background `ffmpeg` frame-grab (one frame at 1s in, scaled to 256px),
  written into that same cache directory so it's picked up by any other
  freedesktop-compliant file manager too. Runs off the main thread
  (`grid_view.rs` spawns it per video item and updates the `ItemObject`'s
  `thumbnail-path` property when done, via a weak ref so a row scrolled
  away from doesn't keep anything alive) — folders full of video no
  longer stall on open. No-op if `ffmpeg` isn't installed.
- **"Connect to Server."** The sidebar showed already-mounted network
  shares but had no way to mount a *new* `smb://` / `sftp://` / `ftp://`
  location. Added a dialog (`ui/dialogs.rs::show_connect_to_server`) that
  takes an address and mounts it via `gio::File::mount_enclosing_volume`
  with a `gtk::MountOperation` for auth prompts; wired into the sidebar
  as an always-visible "Connect to Server…" row.
- **Conflict dialog** now says how many files conflict, instead of just
  "some files."
- **Default view (grid/list)** is now a persisted setting
  (`config/settings.rs`), not just a session-only toggle — new windows
  open in whichever you last picked in Settings.
- **The real XDG Desktop Portal `FileChooser` interface**
  (`src/portal/xdg_portal.rs`, new file). The custom `org.mitos.FilePicker`
  interface from the last pass is untouched and still works exactly as
  before — this *adds* a second, spec-compliant object at
  `/org/freedesktop/portal/desktop`, reusing the same dialog-showing
  channel. This is the piece the last pass deliberately skipped as
  needing its own focus; it's the least certain code in this session,
  specifically:
  - `option_bool()` / the `current_name` extraction use zvariant's
    `Value`/`OwnedValue` conversions as best-recalled, not verified.
  - `Connection::emit_signal`'s exact generic signature (the
    `Option::<&str>::None` destination argument in particular).
  - `SaveFiles` is intentionally not implemented (returns
    `NotSupported`, which is spec-legal) — see the doc comment at the
    top of the file for why.
  - Everything else in that file (the Request/Response object lifecycle,
    the path scheme, reusing the existing `PortalRequest` channel) is
    higher-confidence.

## Two sessions ago

Fixed CI warnings and wired up dead code: the `desktop` module wasn't
declared in `main.rs` (~800 lines dead), "Set Default App" was a
hardcoded stub now backed by real `.desktop`-file / MIME-association
writing, theme changes now live-reload via the config watcher instead of
needing a restart, and the zip/tar archive extractor was checked for
zip-slip path traversal (it was already safe — verified, not patched).

Known gap carried over from that pass, now closed above: the portal only
implemented `org.mitos.FilePicker`, not the standard
`org.freedesktop.portal.FileChooser`.

Still not touched: ~40 `GTK4 Dialog` deprecation warnings (`Dialog`
itself is deprecated in favor of `AlertDialog`/custom windows in newer
GTK4; out of scope for a warnings-only pass).
