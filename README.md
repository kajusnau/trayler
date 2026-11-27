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
| `--control PATH`              | Unix socket on which trayler accepts commands; see [Runtime control](#runtime-control)                                |
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
| `label`              | `id`, `label`, `icon` | a line of text that cannot be clicked |
| `separator`          | none                                    | a separator line                                                  |

`control` (the same as `--control`) can also be set at the top level of the config. `[exit]` takes `label`, `icon`, `id` (default `exit`) and `signal` (same values as `--exit-signal`, only with `--pid` or a command). With
`--config -` the config is read from stdin, so a command started after `--` gets an empty stdin.

When the session ends through a menu entry, trayler prints that entry's `id` on stdout (after its own PID when it printed one). Nothing is printed when the followed process ends by itself or trayler is stopped by a signal. Exit codes are unchanged.

## Runtime control

With `--control PATH` (or `control` in the config), trayler listens on a Unix socket that only your user can use, and removes it on exit. It refuses a path that is a file or a socket another trayler is using. Send a command with `trayler ctl PATH COMMAND...`; it prints nothing on success and exits 1 with the error on stderr otherwise. The followed command and every menu command get the path in `TRAYLER_CONTROL`.

| Command                          | Effect                                                                                       |
| -------------------------------- | -------------------------------------------------------------------------------------------- |
| `icon NAME`                      | change the icon                                                                              |
| `title TEXT`                     | change the title and tooltip                                                                 |
| `set ID KEY=VALUE...`            | change `label`, `icon`, `checked`, `label_checked`, `icon_checked` or `command` of an entry; for the exit entry only `label` and `icon` |
| `add KEY=VALUE... [after=ID\|before=ID]` | add an entry, just above Exit unless `after` or `before` is given; it needs an `id`             |
| `remove ID`                      | remove an entry; the exit entry cannot be removed                                            |

`set` only changes the entry; it never runs its command. Values are plain words separated by spaces. Wrap a word in double quotes to include spaces, and escape `"` and `\` inside as `\"` and `\\`. Words cannot contain line breaks. Each command gets one reply line on the socket: `ok` or `error: MESSAGE`.

```sh
trayler --config recording.toml --control "$XDG_RUNTIME_DIR/rec.sock" --pid "$pid" &
trayler ctl "$XDG_RUNTIME_DIR/rec.sock" set status "label=Paused"
```

A menu command can update its own entry:

```toml
command = 'gsr pause $1 && trayler ctl "$TRAYLER_CONTROL" icon media-playback-pause-symbolic'
```

## Building

```sh
nix build        # or: cargo build --release
```

Flakes can add `trayler` to their package set with the overlay:

```nix
nixpkgs.overlays = [ inputs.trayler.overlays.default ];
```

## License

Licensed under the [GNU General Public License v3.0](LICENSE) or, at your option, any later version.
