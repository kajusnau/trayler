# trayler

A tiny tray icon that tags along with a process.

Give `trayler` an icon, a few menu entries and, optionally, the PID of something that is running. It puts the icon
in your system tray; click it and you get your entries plus **Exit**. When the process ends, the icon quietly goes
with it.

It was made for screen recording: show a "Recording" icon while the recorder runs, with a **Stop recording** entry
right where you would look for it. Anything that runs in the background and deserves a visible off switch fits
just as well.

## Usage

```sh
gpu-screen-recorder -w portal -o capture.mkv &
trayler --icon media-record-symbolic --title Recording --pid $! \
        --option "media-playback-stop-symbolic:Stop recording:pkill -INT -f gpu-screen-recorder" &
```

| Flag | Meaning |
| --- | --- |
| `--icon NAME` | required; icon theme name, such as `media-record-symbolic` |
| `--title TEXT` | name and tooltip of the icon (default `trayler`) |
| `--pid PID` | process to follow; **Exit** sends it SIGTERM, and the icon disappears when it ends |
| `--option ICON:LABEL:COMMAND` | menu entry with an optional icon (`:Label:command` for none); the command runs with `sh -c`. Repeat for more entries |
| `--exit-label TEXT` | text of the last menu entry (default `Exit`) |
| `--exit-signal SIGNAL` | signal the last entry sends with `--pid`: `TERM` (default), `INT`, `HUP`, `QUIT`, `KILL`, `USR1`, `USR2` |

Without `--pid`, trayler prints its own PID and stays until you choose **Exit** or stop it with `kill`. If the followed process ignores SIGTERM, **Exit** leaves it, and trayler, running.

trayler needs a tray that speaks StatusNotifierItem, as COSMIC, KDE Plasma and most Wayland panels do. GNOME is not
supported.

## Building

```sh
nix build        # or: cargo build --release
```

## License

Licensed under the [GNU General Public License v3.0](LICENSE) or, at your option, any later version.
