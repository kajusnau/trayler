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

struct Trayler {
    icon: String,
    title: String,
    options: Vec<MenuOption>,
    quit: mpsc::UnboundedSender<()>,
    pidfd: Option<Arc<OwnedFd>>,
}

impl Trayler {
    fn exit(&self) {
        match &self.pidfd {
            // trayler exits once the pidfd reports the process gone.
            Some(pidfd) => {
                if let Err(e) = pidfd_send_signal(pidfd, Signal::TERM) {
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
                label: "Exit".into(),
                activate: Box::new(|this: &mut Self| this.exit()),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;

    let pidfd = match cli.pid {
        Some(raw) => Some(Arc::new(open_pidfd(raw)?)),
        None => {
            println!("{}", std::process::id());
            // stdout is block-buffered when piped; callers read the PID right away.
            std::io::stdout().flush()?;
            None
        }
    };

    let (quit, mut quit_rx) = mpsc::unbounded_channel();
    let handle = Trayler {
        icon: cli.icon,
        title: cli.title,
        options: cli.options,
        quit,
        pidfd: pidfd.clone(),
    }
    .spawn()
    .await?;

    let process_exit = async {
        match pidfd {
            // A pidfd becomes readable when its process exits.
            Some(pidfd) => {
                // SAFETY: the AsyncFd owns the Arc, so the fd stays open while registered.
                let fd = unsafe { AsyncFd::register(pidfd) }?;
                fd.readable().await.map(|_| ())
            }
            None => std::future::pending().await,
        }
    };

    tokio::select! {
        result = process_exit => result?,
        _ = quit_rx.recv() => {}
        _ = term.recv() => {}
        _ = int.recv() => {}
    }

    handle.shutdown().await;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = parse_cli();
    if let Err(e) = run(cli).await {
        eprintln!("trayler: {e}");
        std::process::exit(1);
    }
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
}
