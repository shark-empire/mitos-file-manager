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
