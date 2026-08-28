# Themesync bar widget

The `themesync` daemon in the Omarchy bar. `themesync install` copies these two files to
`~/.config/omarchy/plugins/io.github.ncr.themesync/` and enables the widget; they are also
carried inside the binary (`host/src/setup.rs`), so this directory is the source and the
installed copy is a product.

- `manifest.json` — the plugin manifest (`kinds: ["bar-widget"]`, two settings: refresh
  interval, hide while the daemon is not running).
- `Panel.qml` — the bar mark and the panel. It speaks the daemon's socket protocol
  directly (`host/src/transport/ipc.rs`: one JSON line per connection): `status` every
  refresh, `sync` / `push_list` / `reset_counter` from the buttons.

To work on it: `ln -s $PWD/shell/io.github.ncr.themesync ~/.config/omarchy/plugins/` and
`omarchy-shell shell rescanPlugins`; the shell reloads the plugin on every save
(`themesync install` leaves a symlink alone). Omarchy's validator rejects symlinks, so the
release path is always the copy.
