# MITOS File Manager Integration

This is the default graphical file manager for MITOS, built with GTK4. 

## How it connects to the MITOS Ecosystem

### 1. Desktop Notifications (D-Bus)
This application uses standard `gio::Notification` to report job statuses (e.g., "Copy complete", "Extraction failed"). 
Because `mitos-gui` owns the `org.freedesktop.Notifications` D-Bus service, `gio` automatically routes these notifications to the compositor's GPU-accelerated notification engine. No custom IPC is required.

### 2. XDG Desktop Portal
The `src/portal/` module implements the `org.freedesktop.portal.FileChooser` interface. This allows sandboxed applications (e.g., Flatpaks) to securely request file open/save dialogs through the native MITOS file picker rather than falling back to generic fallback UIs.

### 3. Theme Syncing
As a GTK4 application, this file manager automatically inherits the system theme. When a user changes the theme in `mitos-settings`, the settings daemon updates `~/.config/mitos/home.conf`. This application's `config::settings` module watches that file and applies the new GTK4 CSS providers live.

### 4. Hotplug Support
The application uses `gio::VolumeMonitor` to listen for USB and network drive hotplug events, automatically updating the sidebar and redirecting the user to their home directory if a currently-viewed mounted volume is unplugged.

### 5. `org.mitos.FilePicker` (session bus)
A simpler, MITOS-only alternative to the XDG portal. Object path `/org/mitos/FilePicker`, well-known name `org.mitos.FilePicker`. Every method blocks until the user answers and returns the chosen absolute paths as `as`:

| Method | Arguments | Returns |
|---|---|---|
| `OpenFile` | `title: s` | `as` — the chosen file |
| `SaveFile` | `title: s`, `default_name: s` | `as` — the chosen save path |
| `OpenFolder` | `title: s` | `as` — the chosen folder |
| `SaveFiles` | `title: s`, `files: as` | `as` — the chosen folder joined with each name in `files` |

An **empty array** means the user cancelled. A D-Bus error (`org.freedesktop.DBus.Error.Failed`) means the picker couldn't produce a usable answer -- for example the user picked a location that has no local path (an `sftp://` share with no FUSE mount).

The XDG `FileChooser` portal above maps the same three outcomes onto the `Response` signal: `0` = selected (`uris`), `1` = cancelled, `2` = failed.

### 6. `org.mitos.Desktop` (session bus)
Object path `/org/mitos/Desktop`, name `org.mitos.Desktop`. Lets `mitos-gui` / `mitos-settings` drive desktop-level settings through the file manager:

| Method | Arguments | Effect |
|---|---|---|
| `SetWallpaper` | `path: s` | Writes `wallpaper = <path>` into `~/.config/mitos/home.conf` |
| `GetWallpaper` | — | Returns the current `wallpaper` value (`s`, empty = default) |
| `OpenDesktopSettings` | — | Spawns `mitos-settings` |

### 7. Files this app reads and writes
| File | Owner | Purpose |
|---|---|---|
| `~/.config/mitos/home.conf` (`$XDG_CONFIG_HOME` honoured) | shared with `mitos-gui` | `key = value` lines: `theme_mode`, `accent_color`, `glass_opacity`, `panel_radius`, `wallpaper`, `show_hidden_files`, `enable_thumbnails`, `thumbnail_max_mb`. Watched live -- editing it re-themes and refreshes every open window. |
| `~/.config/mitos/file-manager/settings.json` | this app | `confirm_trash`, `default_view` (`grid`/`list`), plus a copy of the shared keys |
| `~/.config/mitos/file-manager/bookmarks.json` | this app | Sidebar bookmarks, `[{ "name", "path" }]` |
| `~/.config/mitos/file-manager/plugins/<name>/plugin.json` | third parties | Context-menu plugins (below) |

### 8. Context-menu plugins
Drop a folder containing `plugin.json` into `~/.config/mitos/file-manager/plugins/`:

```json
{
  "name": "Open Terminal",
  "version": "1.0.0",
  "description": "Opens a terminal in the selected folder",
  "author": "MITOS",
  "actions": [
    {
      "id": "open_terminal",
      "label": "Open Terminal Here",
      "command": "cd {files} && x-terminal-emulator",
      "applies_to": "folders"
    }
  ]
}
```

`applies_to` is `files`, `folders` or `both`. The command runs through `sh -c` with `{files}` replaced by the selected paths, **each single-quoted**, so names containing spaces or shell metacharacters arrive as one literal argument each. Actions are re-read every time the menu opens; no restart needed.

### 9. Command line
`mitos-file-manager [PATH...]` opens each directory in its own tab. A file path opens the folder that contains it. With no arguments it opens the home directory.

### 10. `org.mitos.Trash` (session bus)
The trash can as a service, so the desktop shell can show a full/empty icon, a settings panel can offer "Empty Trash", and other apps can list or restore what was deleted. Object path `/org/mitos/Trash`, name `org.mitos.Trash`. It reads the user's trash plus the per-drive trash folders of anything mounted under `/media`, `/run/media` or `/mnt`.

| Method | Arguments | Returns |
|---|---|---|
| `Count` | — | `u` — number of trashed items |
| `IsEmpty` | — | `b` |
| `List` | — | `a(ssss)` — `(file_path, original_path, deletion_date, location)` per item; `deletion_date` is empty when unknown, `location` is "Home" or the drive's name |
| `Restore` | `file_path: s` | — puts the item back where it came from |
| `DeleteForever` | `file_path: s` | — |
| `Empty` | — | — |

Items are identified by their `file_path` from `List`. The service only acts on paths it currently lists, so it cannot be used to delete anything else. Failures come back as `org.freedesktop.DBus.Error.Failed` with a message.

### 11. Administrator operations
The file manager never runs as root. When an operation fails with "Permission denied" the user is offered *Retry as Administrator*, which re-runs this same executable in a window-less helper mode:

```text
<elevation command> /path/to/mitos-file-manager --privileged <operation> <arguments...>
```

* **Elevation command** -- `pkexec` by default. Set `MITOS_ELEVATE_COMMAND` (split on whitespace, e.g. `mitosvc-ctl elevate --`) to route the prompt through another mechanism, such as the MITOS permission service, without recompiling.
* **Operations** -- a fixed list; every path must be absolute and free of `..`:

| Arguments after `--privileged` | Effect |
|---|---|
| `delete <path>...` | Delete files / folder trees (links removed, never followed) |
| `rename <from> <to>` | Rename; refuses to replace an existing file |
| `mkdir <path>` / `touch <path>` | Create a folder / an empty file; never overwrite |
| `chmod <octal> <path>` | Set permission bits |
| `paste <copy\|move> (<keep\|replace> <source> <target>)...` | Copy or move, keeping both or replacing on a name clash |

Exit status 0 on success; otherwise 1 with a one-line reason on stderr. `delete`, `rename` and `move` refuse the same protected paths (`/usr`, `/etc`, mount points, ...) that the GUI does.

### 12. Clipboard and drag-and-drop
* **Copy / Cut** puts three formats on the system clipboard: a file list (`text/uri-list`), `x-special/gnome-copied-files` (`copy` or `cut`, then the URIs -- how Nautilus, Thunar and PCManFM tell a cut from a copy) and the paths as plain text.
* **Paste** reads the file list. If the clipboard still holds the files this app put there, its own copy-vs-cut applies; files from another program are pasted as a copy.
* **Drop targets** accept a file list and offer copy or move. Holding Ctrl forces a copy and Shift a move; with neither, the same filesystem moves and a different one copies. Dropping onto the Trash row trashes the files properly.

### 13. Caches and state
| Path | Purpose |
|---|---|
| `~/.cache/thumbnails/normal/` | Freedesktop thumbnail cache. Images and videos are rendered here at 128 px with `Thumb::URI` and `Thumb::MTime`, so other file managers reuse them; one older than its file is regenerated |
| `~/.config/mitos/file-manager/servers.json` | Recently used network addresses (never with a password) |
| `recently-used.xbel` (via `GtkRecentManager`) | Files opened from this app are added; read back for the sidebar's "Recent" section |
