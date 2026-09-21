# MITOS File Manager -- specification coverage

Status of every item in the "Complete Specification". **Done** means it is
implemented and wired into the UI; where a feature depends on something the
system must provide, the note says so. Nothing here has been run on a
display yet (see `CHANGES.md`): the logic underneath is covered by unit
tests, the GTK layer is hand-verified.

## Core

| Item | Status | Where / notes |
|---|---|---|
| Filesystem browsing, folders, files | Done | Icon, list and tree views (`ui/grid_view.rs`, `list_view.rs`, `tree_view.rs`) |
| Drives | Done | Sidebar "Devices": mounted drives, plus present-but-unmounted ones (click to mount) -- `ui/sidebar.rs` |
| Removable devices | Done | Eject (not just unmount) for ejectable media |
| Network locations | Done | "Connect to Server..." (smb/sftp/ftp/...), remembered servers, a "Network" sidebar section. Browsing a share needs GVfs's FUSE mount to give it a local path |
| Bookmarks | Done | Add/remove (button toggles), persisted |
| Recent files | Done | Recorded whenever something is opened; "Clear Recent Files" on right-click |
| Search | Done | Name, contents, file type, recursive; cancellable |
| File previews | Done | Images (scaled decode), text, video frame, archive contents, folder summary |
| Thumbnails | Done | Freedesktop cache, generated in the background for images and videos |
| Metadata / Properties | Done | Size (folders measured in the background), times, owner, group, mode, link target, your access |
| Permissions | Done | View and edit (chmod) in Properties |

## File operations

| Item | Status | Where / notes |
|---|---|---|
| Open / Open With | Done | Default app per MIME type; picker |
| Create (folder, file) | Done | Never overwrites; names validated |
| Rename / batch rename | Done | Never replaces an existing file; batch has preview + rollback |
| Copy / move | Done | Job engine, plus system clipboard and drag-and-drop |
| Delete -> Trash, restore | Done | Trash view: restore, delete forever, empty |
| Permanent delete | Done | Shift+Delete or context menu; always confirms |
| Duplicate | Done | Ctrl+D |
| Compress | Done | ZIP and TAR.GZ |
| Extract | Done | ZIP, TAR, TAR.GZ natively; TAR.BZ2/XZ/ZST, 7Z, RAR via `bsdtar` or `7z` if installed |
| Link creation | Done | "Create Link" (symbolic link) |
| Batch operations | Done | Multi-select for every operation |
| Progress, cancel, pause | Done | Progress dialog with Cancel / Pause |
| Conflict resolution | Done | Replace / Skip / Keep both |

## Navigation

| Item | Status | Where / notes |
|---|---|---|
| Tabs, split view | Done | Drag files between panes, tabs and the sidebar |
| Breadcrumbs, path entry | Done | Typing a file path opens the file |
| Back / forward, history | Done | Right-click Back or Forward for a list of places |
| Favorites | Done | Called "Bookmarks" in the UI |
| Keyboard navigation | Done | See the shortcuts dialog (`?` button) |
| Type-ahead search | Done | Just start typing |

## Integration

| Item | Status | Where / notes |
|---|---|---|
| MIME associations, default applications | Done | Settings -> recommended defaults, per-type "Open With" |
| File chooser, save/open dialogs | Done | `org.freedesktop.portal.FileChooser` and `org.mitos.FilePicker` |
| Drag-and-drop | Done | Copy or move by modifier key, else by filesystem; onto views, tabs, sidebar, split pane, Trash |
| Clipboard | Done | System clipboard: file list, `x-special/gnome-copied-files`, plain text |
| Archive support | Done | See Compress / Extract; preview lists contents |
| Mount / unmount / eject | Done | Sidebar |
| Network shares | Done | See Network locations |
| Trash service | Done | `org.mitos.Trash` on the session bus (`INTEGRATION.md`) |

## Security

| Item | Status | Where / notes |
|---|---|---|
| Permission awareness | Done | Read-only folders grey out New/Paste and are labelled; Properties shows what *you* can do |
| Sandbox / portal compatibility | Done | Speaks the FileChooser portal. The administrator helper needs the host's `pkexec` and is not for use inside a sandbox |
| Protected system paths | Done | `/`, `/usr`, `/etc`, ..., your home and standard folders, and mount points can't be deleted, moved or renamed (`filesystem/protection.rs`) |
| Privileged-operation prompts | Done | "Permission denied" offers *Retry as Administrator* (pkexec, or `MITOS_ELEVATE_COMMAND`) -- `operations/privileged.rs` |
| Symlink safety | Done | Never followed when deleting, measuring, searching or archiving; extraction rejects paths that escape the folder; setuid bits stripped from extracted files |
| Safe deletion | Done | Trash by default; permanent delete confirms, refuses protected paths, warns in system locations |

## Performance

| Item | Status | Where / notes |
|---|---|---|
| Asynchronous directory loading | Done | Listing runs on a worker thread; stale results are discarded (`main.rs: start_directory_load`) |
| Thumbnail caching | Done | `mime/thumbnail.rs`: 128 px PNGs, freshness-checked, two shared workers |
| Lazy metadata | Done | One `lstat` per file, MIME from the name, icons memoised, thumbnails only for visible rows |
| Large-directory optimisation | Done | Chunked population, single-signal store updates, no duplicate copy of the listing |
| Background copy engine | Done | `operations/jobs.rs`: chunked, cancellable, keeps timestamps, checks free space first |
