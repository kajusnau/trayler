# trayler

A configurable tray icon and menu for any process.

Programs such as screen recorders, long builds or sync scripts often run with no tray presence at all. `trayler`
gives them one: point it at a running process with `--pid`, or let it start a command given after `--`, and it
trails that process with a tray icon and menu entries from a small TOML config, plus **Exit**. **Exit** stops the
process, and when the process ends on its own, the icon goes with it.

## Usage

```sh
trayler --config recording.toml --exit-signal INT \
        -- gpu-screen-recorder -w portal -o capture.mkv
```

| Flag                          | Meaning                                                                                                               |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `--config FILE`               | TOML file with the menu and settings; `-` reads stdin                                                                 |
| `--icon NAME`                 | icon theme name, such as `media-record-symbolic`; required here or in the config                                      |
| `--title TEXT`                | name and tooltip of the icon (default `trayler`)                                                                      |
| `--pid PID`                   | process to follow; **Exit** sends it the exit signal, and the icon disappears when it ends                            |
| `-- COMMAND [ARGS]`           | run the command and follow it; **Exit** signals it, stopping trayler stops it, and trayler exits with its exit code   |
| `--exit-label TEXT`           | text of the last menu entry (default `Exit`)                                                                          |
| `--exit-signal SIGNAL`        | signal the last entry sends with `--pid` or a command: `TERM` (default), `INT`, `HUP`, `QUIT`, `KILL`, `USR1`, `USR2` |

Without `--pid` or a command, trayler prints its own PID and stays until you choose **Exit** or stop it with `kill`. If the followed process ignores the exit signal, **Exit** leaves it, and trayler, running.

Flags override the same settings in the config.

trayler needs a tray that speaks StatusNotifierItem, as COSMIC, KDE Plasma and most Wayland panels do. GNOME is not
supported.

## Config

`trayler init` writes a commented example (the same as `example.toml` in the repository) to `./trayler.toml` (or the path you give; `-` prints it). It never
overwrites a file.

```toml
icon = "media-record-symbolic"
title = "Recording"

[exit]
label = "Stop"

[[entry]]
label = "Open Videos"
icon = "folder-videos-symbolic"
command = "xdg-open ~/Videos"

[[entry]]
type = "toggle"
label = "Do not disturb"
command = "notify-send \"Do not disturb: $1\""
```

The menu lists the entries top to bottom as they are written in the config, then **Exit**. Each entry has a
`type`:

| Type                 | Keys                                    | Behaviour                                                         |
| -------------------- | --------------------------------------- | ----------------------------------------------------------------- |
| `button` (default)   | `id`, `label`, `icon`, `command`, `exit` | runs the command with `sh -c`; `exit = true` ends the session like **Exit**, needs an `id`, and makes `command` optional |
| `toggle`             | `id`, `label`, `icon`, `label_checked`, `icon_checked`, `command`, `checked` | checkbox; the command gets `on` or `off` as `$1`; `label_checked`/`icon_checked` are shown while checked |
| `separator`          | none                                    | a separator line                                                  |

`[exit]` takes `label`, `id` (default `exit`) and `signal` (same values as `--exit-signal`, only with `--pid` or a command). With
`--config -` the config is read from stdin, so a command started after `--` gets an empty stdin.

When the session ends through a menu entry, trayler prints that entry's `id` on stdout (after its own PID when it printed one). Nothing is printed when the followed process ends by itself or trayler is stopped by a signal. Exit codes are unchanged.

## Building

```sh
nix build        # or: cargo build --release
```

## License

Licensed under the [GNU General Public License v3.0](LICENSE) or, at your option, any later version.
