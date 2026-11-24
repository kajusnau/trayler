use std::io::Write;
use std::os::fd::OwnedFd;
use std::process::Command;
use std::sync::Arc;

use clap::Parser;
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, TrayMethods};
use rustix::process::{pidfd_open, pidfd_send_signal, Pid, PidfdFlags, Signal};
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

/// A configurable tray icon and menu for any process
#[derive(Parser)]
#[command(
    version,
    after_help = "\
Clicking the icon opens a menu: the --option entries in order, then Exit
(--exit-label). With --pid or a command after --, Exit sends --exit-signal
(default TERM) to that process, and the icon disappears when it ends. A
command is stopped together with trayler, and trayler exits with its exit
code. Without either, trayler prints its own PID and Exit closes it.

Examples:
  trayler --icon media-record-symbolic --title Recording --pid \"$pid\" \\
          --option \"media-playback-stop-symbolic:Stop recording:pkill -INT recorder\" &
  trayler --icon dialog-information --option \":Say hi:notify-send hi\" &
  trayler --icon media-record-symbolic --title Recording --exit-signal INT \\
          -- gpu-screen-recorder -w portal -o capture.mkv",
    group(clap::ArgGroup::new("target").args(["pid", "command"])),
)]
struct Cli {
    /// Icon theme name, for example media-record-symbolic
    #[arg(long, value_name = "NAME")]
    icon: String,

    /// Title and tooltip of the icon
    #[arg(long, value_name = "TEXT", default_value = "trayler")]
    title: String,

    /// Process to follow; with neither it nor a command, trayler prints its own PID and runs until Exit
    #[arg(long)]
    pid: Option<i32>,

    /// Menu entry; ICON may be empty, COMMAND runs with `sh -c`; repeatable
    #[arg(long = "option", value_name = "ICON:LABEL:COMMAND", value_parser = parse_option)]
    options: Vec<MenuOption>,

    /// Text of the last menu entry
    #[arg(long, value_name = "TEXT", default_value = "Exit")]
    exit_label: String,

    /// Signal the last menu entry sends to the followed process or command
    #[arg(long, value_name = "SIGNAL", value_enum, default_value_t = ExitSignal::Term, requires = "target")]
    exit_signal: ExitSignal,

    /// Command to run and follow; trayler exits with its exit code
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct MenuOption {
    icon: String,
    label: String,
    command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
#[value(rename_all = "UPPER")]
enum ExitSignal {
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

fn parse_option(s: &str) -> Result<MenuOption, String> {
    let mut parts = s.splitn(3, ':');
    let (Some(icon), Some(label), Some(command)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err("expected ICON:LABEL:COMMAND".into());
    };
    if label.is_empty() {
        return Err("label is empty".into());
    }
    if command.is_empty() {
        return Err("command is empty".into());
    }
    Ok(MenuOption {
        icon: icon.into(),
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

// ksni reads `_` as an access-key marker; doubling keeps it literal.
fn menu_label(label: &str) -> String {
    label.replace('_', "__")
}

fn spawn_command(command: &str) {
    match Command::new("sh").arg("-c").arg(command).spawn() {
        // Wait on a thread so the finished child is reaped instead of left a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("trayler: {command}: {e}"),
    }
}

fn open_pidfd(raw: i32) -> Result<OwnedFd, String> {
    // Pid::from_raw debug-asserts non-negative, so the sign check must come first.
    let pid =
        if raw > 0 { Pid::from_raw(raw) } else { None }.ok_or(format!("invalid pid {raw}"))?;
    pidfd_open(pid, PidfdFlags::empty()).map_err(|e| format!("pid {raw}: {e}"))
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

struct Trayler {
    icon: String,
    title: String,
    options: Vec<MenuOption>,
    exit_label: String,
    exit_signal: Signal,
    quit: mpsc::UnboundedSender<()>,
    pidfd: Option<Arc<OwnedFd>>,
}

impl Trayler {
    fn exit(&self) {
        match &self.pidfd {
            // trayler exits once the pidfd reports the process gone.
            Some(pidfd) => {
                if let Err(e) = pidfd_send_signal(pidfd, self.exit_signal) {
                    eprintln!("trayler: {e}");
                }
            }
            None => {
                let _ = self.quit.send(());
            }
        }
    }
}

impl ksni::Tray for Trayler {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn icon_name(&self) -> String {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.title.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = self
            .options
            .iter()
            .map(|option| {
                let command = option.command.clone();
                StandardItem {
                    label: menu_label(&option.label),
                    icon_name: option.icon.clone(),
                    activate: Box::new(move |_: &mut Self| spawn_command(&command)),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        if !items.is_empty() {
            items.push(MenuItem::Separator);
        }
        items.push(
            StandardItem {
                label: menu_label(&self.exit_label),
                activate: Box::new(|this: &mut Self| this.exit()),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

async fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;

    let mut child = None;
    let pidfd = if let Some((program, args)) = cli.command.split_first() {
        let mut spawned = tokio::process::Command::new(program)
            .args(args)
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let pid = spawned
            .id()
            .ok_or("command exited before it could be followed")?;
        let pidfd = match open_pidfd(pid as i32) {
            Ok(pidfd) => pidfd,
            Err(e) => {
                // Dropping a tokio Child does not kill it.
                let _ = spawned.start_kill();
                let _ = spawned.wait().await;
                return Err(e.into());
            }
        };
        child = Some(spawned);
        Some(Arc::new(pidfd))
    } else if let Some(raw) = cli.pid {
        Some(Arc::new(open_pidfd(raw)?))
    } else {
        println!("{}", std::process::id());
        // stdout is block-buffered when piped; callers read the PID right away.
        std::io::stdout().flush()?;
        None
    };
    // Only a command trayler started is stopped together with trayler.
    let forward_to = child.as_ref().and(pidfd.clone());

    let (quit, mut quit_rx) = mpsc::unbounded_channel();
    let exit_signal = cli.exit_signal.into();
    let spawned = Trayler {
        icon: cli.icon,
        title: cli.title,
        options: cli.options,
        exit_label: cli.exit_label,
        exit_signal,
        quit,
        pidfd: pidfd.clone(),
    }
    .spawn()
    .await;
    let handle = match spawned {
        Ok(handle) => handle,
        Err(e) => {
            // Without a tray there is no way to stop the command, so do not orphan it.
            if let (Some(mut child), Some(pidfd)) = (child, &forward_to) {
                let _ = pidfd_send_signal(pidfd, exit_signal);
                let _ = child.wait().await;
            }
            return Err(e.into());
        }
    };

    let followed = async {
        match (child.as_mut(), pidfd) {
            (Some(child), _) => child.wait().await.map(exit_code),
            // A pidfd becomes readable when its process exits.
            (None, Some(pidfd)) => {
                // SAFETY: the AsyncFd owns the Arc, so the fd stays open while registered.
                let fd = unsafe { AsyncFd::register(pidfd) }?;
                fd.readable().await.map(|_| 0)
            }
            (None, None) => std::future::pending().await,
        }
    };
    tokio::pin!(followed);

    let forward = |signal: Signal| match &forward_to {
        Some(pidfd) => {
            if let Err(e) = pidfd_send_signal(pidfd, signal) {
                eprintln!("trayler: {e}");
            }
            true
        }
        None => false,
    };

    let code = loop {
        tokio::select! {
            result = &mut followed => break result?,
            _ = quit_rx.recv() => break 0,
            _ = term.recv() => if !forward(Signal::TERM) { break 0 },
            _ = int.recv() => if !forward(Signal::INT) { break 0 },
        }
    };

    handle.shutdown().await;
    Ok(code)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = parse_cli();
    match run(cli).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("trayler: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(icon: &str, label: &str, command: &str) -> MenuOption {
        MenuOption {
            icon: icon.into(),
            label: label.into(),
            command: command.into(),
        }
    }

    #[test]
    fn splits_icon_label_and_command() {
        assert_eq!(
            parse_option("media-playback-stop-symbolic:Stop recording:pkill -INT recorder"),
            Ok(option(
                "media-playback-stop-symbolic",
                "Stop recording",
                "pkill -INT recorder"
            ))
        );
    }

    #[test]
    fn allows_an_empty_icon() {
        assert_eq!(
            parse_option(":Open Videos:xdg-open ~/Videos"),
            Ok(option("", "Open Videos", "xdg-open ~/Videos"))
        );
    }

    #[test]
    fn keeps_colons_in_the_command() {
        assert_eq!(
            parse_option(":Open site:xdg-open https://example.com:8080"),
            Ok(option("", "Open site", "xdg-open https://example.com:8080"))
        );
    }

    #[test]
    fn rejects_fewer_than_two_colons() {
        assert!(parse_option("Stop recording:pkill recorder").is_err());
        assert!(parse_option("Stop recording").is_err());
    }

    #[test]
    fn rejects_empty_label() {
        assert!(parse_option("icon::pkill -INT recorder").is_err());
    }

    #[test]
    fn rejects_empty_command() {
        assert!(parse_option("icon:Stop recording:").is_err());
    }

    #[test]
    fn escapes_underscores_in_labels() {
        assert_eq!(menu_label("save_as_mp4"), "save__as__mp4");
        assert_eq!(menu_label("Stop recording"), "Stop recording");
    }

    #[test]
    fn opens_a_running_process() {
        assert!(open_pidfd(std::process::id() as i32).is_ok());
    }

    #[test]
    fn rejects_a_missing_process() {
        // PIDs above pid_max (at most 2^22) never exist.
        assert!(open_pidfd(i32::MAX).is_err());
    }

    #[test]
    fn rejects_non_positive_pids() {
        assert!(open_pidfd(0).is_err());
        assert!(open_pidfd(-1).is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("trayler").chain(args.iter().copied()))
    }

    #[test]
    fn exit_defaults() {
        let cli = parse(&["--icon", "x"]).unwrap();
        assert_eq!(cli.exit_label, "Exit");
        assert_eq!(cli.exit_signal, ExitSignal::Term);
    }

    #[test]
    fn accepts_exit_signal_with_pid() {
        let cli = parse(&["--icon", "x", "--pid", "1", "--exit-signal", "INT"]).unwrap();
        assert_eq!(cli.exit_signal, ExitSignal::Int);
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
    fn exit_code_passes_through_the_command_code() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(3 << 8)), 3);
    }

    #[test]
    fn exit_code_maps_signals_like_a_shell() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(15)), 143);
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
}
