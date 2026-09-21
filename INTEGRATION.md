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
