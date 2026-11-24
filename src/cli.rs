use std::path::PathBuf;

use clap::Parser;
use rustix::process::Signal;
use serde::Deserialize;

/// A configurable tray icon and menu for any process
#[derive(Parser)]
#[command(
    version,
    after_help = "\
Clicking the icon opens a menu: the entries from --config in order, then
Exit (--exit-label). With --pid or a command after --, Exit sends
--exit-signal (default TERM) to that process, and the icon disappears when
it ends. A command is stopped together with trayler, and trayler exits with
its exit code. Without either, trayler prints its own PID and Exit closes it.
Flags override the same settings in the config.

Examples:
  trayler --icon media-record-symbolic --title Recording --pid \"$pid\" &
  trayler --config recording.toml --exit-signal INT \\
          -- gpu-screen-recorder -w portal -o capture.mkv",
    group(clap::ArgGroup::new("target").args(["pid", "command"])),
    args_conflicts_with_subcommands = true,
    subcommand_value_name = "ACTION",
    subcommand_help_heading = "Actions",
)]
pub(crate) struct Cli {
    /// TOML file with the menu and settings; - reads stdin (see trayler init)
    #[arg(long, value_name = "FILE")]
    pub(crate) config: Option<PathBuf>,

    /// Icon theme name, for example media-record-symbolic; required here or in the config
    #[arg(long, value_name = "NAME")]
    pub(crate) icon: Option<String>,

    /// Title and tooltip of the icon [default: trayler]
    #[arg(long, value_name = "TEXT")]
    pub(crate) title: Option<String>,

    /// Process to follow; with neither it nor a command, trayler prints its own PID and runs until Exit
    #[arg(long)]
    pub(crate) pid: Option<i32>,

    /// Text of the last menu entry [default: Exit]
    #[arg(long, value_name = "TEXT")]
    pub(crate) exit_label: Option<String>,

    /// Signal the last menu entry sends to the followed process or command [default: TERM]
    #[arg(long, value_name = "SIGNAL", value_enum, requires = "target")]
    pub(crate) exit_signal: Option<ExitSignal>,

    /// Command to run and follow; trayler exits with its exit code
    #[arg(last = true, value_name = "COMMAND")]
    pub(crate) command: Vec<String>,

    #[command(subcommand)]
    pub(crate) action: Option<Action>,
}

#[derive(clap::Subcommand)]
pub(crate) enum Action {
    /// Write a commented example config; - prints it instead
    Init {
        #[arg(value_name = "PATH", default_value = "trayler.toml")]
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum, Deserialize)]
#[value(rename_all = "UPPER")]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum ExitSignal {
    Term,
    Int,
    Hup,
    Quit,
    Kill,
    Usr1,
    Usr2,
}

impl From<ExitSignal> for Signal {
    fn from(signal: ExitSignal) -> Self {
        match signal {
            ExitSignal::Term => Signal::TERM,
            ExitSignal::Int => Signal::INT,
            ExitSignal::Hup => Signal::HUP,
            ExitSignal::Quit => Signal::QUIT,
            ExitSignal::Kill => Signal::KILL,
            ExitSignal::Usr1 => Signal::USR1,
            ExitSignal::Usr2 => Signal::USR2,
        }
    }
}

#[cfg(test)]
pub(crate) fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("trayler").chain(args.iter().copied()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_init_with_a_default_path() {
        let cli = parse(&["init"]).unwrap();
        assert!(
            matches!(cli.action, Some(Action::Init { ref path }) if path == Path::new("trayler.toml"))
        );
        let cli = parse(&["init", "-"]).unwrap();
        assert!(matches!(cli.action, Some(Action::Init { ref path }) if path == Path::new("-")));
    }

    #[test]
    fn init_takes_no_tray_flags() {
        assert!(parse(&["--icon", "x", "init"]).is_err());
    }

    #[test]
    fn exit_flags_are_unset_by_default() {
        let cli = parse(&["--icon", "x"]).unwrap();
        assert_eq!(cli.exit_label, None);
        assert_eq!(cli.exit_signal, None);
    }

    #[test]
    fn accepts_exit_signal_with_pid() {
        let cli = parse(&["--icon", "x", "--pid", "1", "--exit-signal", "INT"]).unwrap();
        assert_eq!(cli.exit_signal, Some(ExitSignal::Int));
    }

    #[test]
    fn rejects_exit_signal_without_a_target() {
        assert!(parse(&["--icon", "x", "--exit-signal", "INT"]).is_err());
    }

    #[test]
    fn rejects_unknown_exit_signals() {
        assert!(parse(&["--icon", "x", "--pid", "1", "--exit-signal", "SIGTERM"]).is_err());
    }

    #[test]
    fn accepts_a_command_after_double_dash() {
        let cli = parse(&["--icon", "x", "--exit-signal", "INT", "--", "sleep", "5"]).unwrap();
        assert_eq!(cli.command, ["sleep", "5"]);
    }

    #[test]
    fn rejects_pid_together_with_a_command() {
        assert!(parse(&["--icon", "x", "--pid", "1", "--", "sleep", "5"]).is_err());
    }

    #[test]
    fn maps_exit_signals() {
        assert_eq!(Signal::from(ExitSignal::Usr1), Signal::USR1);
        assert_eq!(Signal::from(ExitSignal::Kill), Signal::KILL);
    }

    #[test]
    fn rejects_the_removed_menu_flags() {
        assert!(parse(&["--icon", "x", "--option", ":A:a"]).is_err());
        assert!(parse(&["--icon", "x", "--toggle", ":A:a"]).is_err());
    }
}
