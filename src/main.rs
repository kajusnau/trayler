use clap::Parser;

/// A configurable tray icon and menu for any process
#[derive(Parser)]
#[command(
    version,
    after_help = "\
Clicking the icon opens a menu: the --option entries in order, then Exit.
With --pid, Exit sends SIGTERM to that process, and the icon disappears when
the process ends. Without --pid, trayler prints its own PID and Exit closes it.

Examples:
  trayler --icon media-record-symbolic --title Recording --pid \"$pid\" \\
          --option \"Stop recording=pkill -INT recorder\" &
  trayler --icon dialog-information --option \"Say hi=notify-send hi\" &"
)]
struct Cli {
    /// Icon theme name, for example media-record-symbolic
    #[arg(long, value_name = "NAME")]
    icon: String,

    /// Title and tooltip of the icon
    #[arg(long, value_name = "TEXT", default_value = "trayler")]
    title: String,

    /// Process to follow; without it trayler prints its own PID and runs until Exit
    #[arg(long)]
    pid: Option<i32>,

    /// Menu entry, run with `sh -c` when chosen; repeatable
    #[arg(long = "option", value_name = "LABEL=COMMAND", value_parser = parse_option)]
    options: Vec<MenuOption>,
}

#[derive(Clone, Debug, PartialEq)]
struct MenuOption {
    label: String,
    command: String,
}

fn parse_option(s: &str) -> Result<MenuOption, String> {
    let (label, command) = s.split_once('=').ok_or("expected LABEL=COMMAND")?;
    if label.is_empty() {
        return Err("label is empty".into());
    }
    if command.is_empty() {
        return Err("command is empty".into());
    }
    Ok(MenuOption {
        label: label.into(),
        command: command.into(),
    })
}

// clap exits 2 on usage errors; the spec wants 1 for every error.
fn parse_cli() -> Cli {
    Cli::try_parse().unwrap_or_else(|e| {
        let _ = e.print();
        std::process::exit(if e.use_stderr() { 1 } else { 0 });
    })
}

fn main() {
    let _cli = parse_cli();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(label: &str, command: &str) -> MenuOption {
        MenuOption {
            label: label.into(),
            command: command.into(),
        }
    }

    #[test]
    fn splits_label_and_command() {
        assert_eq!(
            parse_option("Stop recording=pkill -INT recorder"),
            Ok(option("Stop recording", "pkill -INT recorder"))
        );
    }

    #[test]
    fn keeps_equals_signs_in_the_command() {
        assert_eq!(
            parse_option("Run=env FOO=bar baz"),
            Ok(option("Run", "env FOO=bar baz"))
        );
    }

    #[test]
    fn rejects_missing_equals() {
        assert!(parse_option("Stop recording").is_err());
    }

    #[test]
    fn rejects_empty_label() {
        assert!(parse_option("=pkill -INT recorder").is_err());
    }

    #[test]
    fn rejects_empty_command() {
        assert!(parse_option("Stop recording=").is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
