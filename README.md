# trayler

A configurable tray icon and menu for any process.

Give `trayler` an icon, a few menu entries and, optionally, a running process's PID or a command to start.
It adds a tray applet; click it and you get your entries plus **Exit**.
When the process ends, the tray applet goes with it.

## Usage

```sh
trayler --icon media-record-symbolic --title Recording --exit-signal INT \
        --option "media-playback-stop-symbolic:Stop recording:pkill -INT -f gpu-screen-recorder" \
        -- gpu-screen-recorder -w portal -o capture.mkv
```

| Flag                          | Meaning                                                                                                               |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `--icon NAME`                 | required; icon theme name, such as `media-record-symbolic`                                                            |
| `--title TEXT`                | name and tooltip of the icon (default `trayler`)                                                                      |
| `--pid PID`                   | process to follow; **Exit** sends it the exit signal, and the icon disappears when it ends                            |
| `-- COMMAND [ARGS]`           | run the command and follow it; **Exit** signals it, stopping trayler stops it, and trayler exits with its exit code   |
| `--option ICON:LABEL:COMMAND` | menu entry with an optional icon (`:Label:command` for none); the command runs with `sh -c`. Repeat for more entries  |
| `--exit-label TEXT`           | text of the last menu entry (default `Exit`)                                                                          |
| `--exit-signal SIGNAL`        | signal the last entry sends with `--pid` or a command: `TERM` (default), `INT`, `HUP`, `QUIT`, `KILL`, `USR1`, `USR2` |

Without `--pid` or a command, trayler prints its own PID and stays until you choose **Exit** or stop it with `kill`. If the followed process ignores the exit signal, **Exit** leaves it, and trayler, running.

trayler needs a tray that speaks StatusNotifierItem, as COSMIC, KDE Plasma and most Wayland panels do. GNOME is not
supported.

## Building

```sh
nix build        # or: cargo build --release
```

## License

Licensed under the [GNU General Public License v3.0](LICENSE) or, at your option, any later version.
