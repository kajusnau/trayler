use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use clap::{CommandFactory, Parser};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip, TrayMethods};
use rustix::process::{pidfd_open, pidfd_send_signal, Pid, PidfdFlags, Signal};
use serde::Deserialize;
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

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
struct Cli {
    /// TOML file with the menu and settings; - reads stdin (see trayler init)
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Icon theme name, for example media-record-symbolic; required here or in the config
    #[arg(long, value_name = "NAME")]
    icon: Option<String>,

    /// Title and tooltip of the icon [default: trayler]
    #[arg(long, value_name = "TEXT")]
    title: Option<String>,

    /// Process to follow; with neither it nor a command, trayler prints its own PID and runs until Exit
    #[arg(long)]
    pid: Option<i32>,

    /// Text of the last menu entry [default: Exit]
    #[arg(long, value_name = "TEXT")]
    exit_label: Option<String>,

    /// Signal the last menu entry sends to the followed process or command [default: TERM]
    #[arg(long, value_name = "SIGNAL", value_enum, requires = "target")]
    exit_signal: Option<ExitSignal>,

    /// Command to run and follow; trayler exits with its exit code
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<String>,

    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(clap::Subcommand)]
enum Action {
    /// Write a commented example config; - prints it instead
    Init {
        #[arg(value_name = "PATH", default_value = "trayler.toml")]
        path: PathBuf,
    },
}

const EXAMPLE: &str = include_str!("../example.toml");

fn init(path: &Path) -> Result<(), String> {
    if path == Path::new("-") {
        print!("{EXAMPLE}");
        return Ok(());
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(EXAMPLE.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum, Deserialize)]
#[value(rename_all = "UPPER")]
#[serde(rename_all = "UPPERCASE")]
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    icon: Option<String>,
    title: Option<String>,
    #[serde(default)]
    exit: ExitConfig,
    #[serde(default, rename = "entry", deserialize_with = "entries")]
    entries: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitConfig {
    label: Option<String>,
    signal: Option<ExitSignal>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum Entry {
    Button {
        label: String,
        #[serde(default)]
        icon: String,
        command: String,
    },
    Toggle {
        label: String,
        #[serde(default)]
        icon: String,
        command: String,
        #[serde(default)]
        checked: bool,
    },
    Separator {},
}

// serde cannot default a missing enum tag, so each entry is read as a table first.
fn entries<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Entry>, D::Error> {
    use serde::de::Error;
    Vec::<toml::Table>::deserialize(deserializer)?
        .into_iter()
        .enumerate()
        .map(|(index, mut table)| {
            let kind = table
                .entry("type")
                .or_insert_with(|| "button".into())
                .as_str()
                .unwrap_or("?")
                .to_owned();
            let fail = |message: String| {
                D::Error::custom(format!("entry {} ({kind}): {message}", index + 1))
            };
            let entry: Entry = toml::Value::Table(table)
                .try_into()
                .map_err(|e: toml::de::Error| fail(e.message().into()))?;
            if let Entry::Button { label, command, .. } | Entry::Toggle { label, command, .. } =
                &entry
            {
                if label.is_empty() {
                    return Err(fail("label is empty".into()));
                }
                if command.is_empty() {
                    return Err(fail("command is empty".into()));
                }
            }
            Ok(entry)
        })
        .collect()
}

fn load_config(name: &str, text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e: toml::de::Error| {
        let at = e.span().map_or(String::new(), |span| {
            let before = &text[..span.start];
            let line = before.matches('\n').count() + 1;
            let column = span.start - before.rfind('\n').map_or(0, |i| i + 1) + 1;
            format!(":{line}:{column}")
        });
        format!("{name}{at}: {}", e.message().trim())
    })
}

fn read_config(path: &Path) -> Result<Config, String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| format!("stdin: {e}"))?;
        return load_config("stdin", &text);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    load_config(&path.display().to_string(), &text)
}

struct Settings {
    icon: String,
    title: String,
    entries: Vec<Entry>,
    exit_label: String,
    exit_signal: ExitSignal,
}

fn settings(cli: &Cli, config: Config) -> Result<Settings, String> {
    if config.exit.signal.is_some() && cli.pid.is_none() && cli.command.is_empty() {
        let file = match &cli.config {
            Some(p) if p == Path::new("-") => "stdin".into(),
            Some(p) => p.display().to_string(),
            None => String::new(),
        };
        return Err(format!("{file}: exit.signal needs --pid or a command"));
    }
    Ok(Settings {
        icon: cli
            .icon
            .clone()
            .or(config.icon)
            .ok_or("an icon is required: --icon or icon in the config")?,
        title: cli
            .title
            .clone()
            .or(config.title)
            .unwrap_or_else(|| "trayler".into()),
        entries: config.entries,
        exit_label: cli
            .exit_label
            .clone()
            .or(config.exit.label)
            .unwrap_or_else(|| "Exit".into()),
        exit_signal: cli
            .exit_signal
            .or(config.exit.signal)
            .unwrap_or(ExitSignal::Term),
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

fn spawn_command(command: &str, state: Option<&str>) {
    let mut sh = Command::new("sh");
    sh.arg("-c").arg(command);
    if let Some(state) = state {
        sh.args(["sh", state]);
    }
    match sh.spawn() {
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
    entries: Vec<Entry>,
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
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                Entry::Button {
                    label,
                    icon,
                    command,
                } => {
                    let command = command.clone();
                    StandardItem {
                        label: menu_label(label),
                        icon_name: icon.clone(),
                        activate: Box::new(move |_: &mut Self| spawn_command(&command, None)),
                        ..Default::default()
                    }
                    .into()
                }
                Entry::Toggle {
                    label,
                    icon,
                    checked,
                    ..
                } => CheckmarkItem {
                    label: menu_label(label),
                    icon_name: icon.clone(),
                    checked: *checked,
                    activate: Box::new(move |this: &mut Self| {
                        if let Entry::Toggle {
                            command, checked, ..
                        } = &mut this.entries[index]
                        {
                            *checked = !*checked;
                            spawn_command(command, Some(if *checked { "on" } else { "off" }));
                        }
                    }),
                    ..Default::default()
                }
                .into(),
                Entry::Separator {} => MenuItem::Separator,
            })
            .collect();
        if items
            .last()
            .is_some_and(|item| !matches!(item, MenuItem::Separator))
        {
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
    let config = match &cli.config {
        Some(path) => read_config(path)?,
        None => Config::default(),
    };
    let settings = settings(&cli, config)?;
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
    let exit_signal = settings.exit_signal.into();
    let spawned = Trayler {
        icon: settings.icon,
        title: settings.title,
        entries: settings.entries,
        exit_label: settings.exit_label,
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
    // Used by the package build; an env var keeps it out of --help and completions.
    match std::env::var("TRAYLER_GENERATE").as_deref() {
        Ok("man") => {
            if let Err(e) = clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout()) {
                eprintln!("trayler: {e}");
                std::process::exit(1);
            }
            return;
        }
        Ok(shell) => {
            let Ok(shell) = shell.parse::<clap_complete::Shell>() else {
                eprintln!("trayler: TRAYLER_GENERATE must be bash, zsh, fish or man");
                std::process::exit(1);
            };
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "trayler",
                &mut std::io::stdout(),
            );
            return;
        }
        Err(_) => {}
    }
    let cli = parse_cli();
    if let Some(Action::Init { path }) = &cli.action {
        if let Err(e) = init(path) {
            eprintln!("trayler: {e}");
            std::process::exit(1);
        }
        return;
    }
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
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("trayler").chain(args.iter().copied()))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("trayler-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("trayler.toml")
    }

    #[test]
    fn the_example_is_a_valid_config() {
        let config = load_config("example.toml", EXAMPLE).unwrap();
        assert_eq!(config.entries.len(), 3);
        assert!(matches!(config.entries[1], Entry::Separator {}));
        assert!(matches!(
            config.entries[2],
            Entry::Toggle { checked: false, .. }
        ));
        assert!(settings(&parse(&[]).unwrap(), config).is_ok());
    }

    #[test]
    fn init_writes_the_example() {
        let path = scratch("writes");
        init(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);
    }

    #[test]
    fn init_does_not_overwrite() {
        let path = scratch("keeps");
        std::fs::write(&path, "mine").unwrap();
        assert!(init(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
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

    fn config(text: &str) -> Result<Config, String> {
        load_config("test.toml", text)
    }

    fn button(label: &str, icon: &str, command: &str) -> Entry {
        Entry::Button {
            label: label.into(),
            icon: icon.into(),
            command: command.into(),
        }
    }

    #[test]
    fn an_entry_without_type_is_a_button() {
        let config = config("[[entry]]\nlabel = \"Hi\"\ncommand = \"echo hi\"\n").unwrap();
        assert_eq!(config.entries, [button("Hi", "", "echo hi")]);
    }

    #[test]
    fn reads_toggles_and_separators_in_order() {
        let config = config(
            "[[entry]]\ntype = \"toggle\"\nlabel = \"T\"\nchecked = true\ncommand = \"t $1\"\n\
             [[entry]]\ntype = \"separator\"\n\
             [[entry]]\nlabel = \"B\"\nicon = \"i\"\ncommand = \"b\"\n",
        )
        .unwrap();
        assert_eq!(
            config.entries,
            [
                Entry::Toggle {
                    label: "T".into(),
                    icon: String::new(),
                    command: "t $1".into(),
                    checked: true,
                },
                Entry::Separator {},
                button("B", "i", "b"),
            ]
        );
    }

    #[test]
    fn toggles_start_unchecked() {
        let config =
            config("[[entry]]\ntype = \"toggle\"\nlabel = \"T\"\ncommand = \"t\"\n").unwrap();
        assert!(matches!(
            config.entries[0],
            Entry::Toggle { checked: false, .. }
        ));
    }

    #[test]
    fn reads_top_level_and_exit_settings() {
        let config =
            config("icon = \"i\"\ntitle = \"T\"\n[exit]\nlabel = \"Stop\"\nsignal = \"INT\"\n")
                .unwrap();
        assert_eq!(config.icon.as_deref(), Some("i"));
        assert_eq!(config.title.as_deref(), Some("T"));
        assert_eq!(config.exit.label.as_deref(), Some("Stop"));
        assert_eq!(config.exit.signal, Some(ExitSignal::Int));
    }

    #[test]
    fn rejects_bad_configs() {
        for text in [
            "colour = \"red\"\n",
            "[exit]\nsignal = \"SIGTERM\"\n",
            "[[entry]]\nlabel = \"B\"\ncommand = \"b\"\nchecked = true\n",
            "[[entry]]\ntype = \"separator\"\nlabel = \"S\"\n",
            "[[entry]]\nlabel = \"B\"\n",
            "[[entry]]\nlabel = \"\"\ncommand = \"b\"\n",
            "[[entry]]\nlabel = \"B\"\ncommand = \"\"\n",
            "[[entry]]\ntype = \"menu\"\nlabel = \"M\"\n",
            "[[entry]]\ntype = \"label\"\nlabel = \"L\"\n",
            "[[entry]]\ntype = \"radio\"\nlabel = \"R\"\ncommand = \"r\"\n",
        ] {
            assert!(config(text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn config_errors_name_the_file_and_line() {
        let error = config("icon = \"i\"\n[[entry]]\nlabel = \"B\"\n").unwrap_err();
        assert!(error.starts_with("test.toml:2:"), "{error}");
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn flags_override_the_config() {
        let cli = parse(&[
            "--icon",
            "flag",
            "--title",
            "flag",
            "--exit-label",
            "flag",
            "--pid",
            "1",
            "--exit-signal",
            "HUP",
        ])
        .unwrap();
        let file = config(
            "icon = \"file\"\ntitle = \"file\"\n[exit]\nlabel = \"file\"\nsignal = \"INT\"\n",
        )
        .unwrap();
        let settings = settings(&cli, file).unwrap();
        assert_eq!(settings.icon, "flag");
        assert_eq!(settings.title, "flag");
        assert_eq!(settings.exit_label, "flag");
        assert_eq!(settings.exit_signal, ExitSignal::Hup);
    }

    #[test]
    fn the_config_alone_supplies_settings() {
        let cli = parse(&["--pid", "1"]).unwrap();
        let file = config(
            "icon = \"file\"\ntitle = \"file\"\n[exit]\nlabel = \"file\"\nsignal = \"INT\"\n",
        )
        .unwrap();
        let settings = settings(&cli, file).unwrap();
        assert_eq!(settings.icon, "file");
        assert_eq!(settings.title, "file");
        assert_eq!(settings.exit_label, "file");
        assert_eq!(settings.exit_signal, ExitSignal::Int);
    }

    #[test]
    fn settings_defaults() {
        let settings = settings(&parse(&["--icon", "x"]).unwrap(), Config::default()).unwrap();
        assert_eq!(settings.title, "trayler");
        assert_eq!(settings.exit_label, "Exit");
        assert_eq!(settings.exit_signal, ExitSignal::Term);
        assert!(settings.entries.is_empty());
    }

    #[test]
    fn an_icon_is_required_somewhere() {
        assert!(settings(&parse(&[]).unwrap(), Config::default()).is_err());
    }

    #[test]
    fn exit_signal_in_the_config_needs_a_target() {
        let file = config("[exit]\nsignal = \"INT\"\n").unwrap();
        let cli = parse(&["--icon", "x", "--config", "menu.toml"]).unwrap();
        let error = settings(&cli, file).err().unwrap();
        assert!(error.starts_with("menu.toml: "), "{error}");
    }

    #[test]
    fn rejects_the_removed_menu_flags() {
        assert!(parse(&["--icon", "x", "--option", ":A:a"]).is_err());
        assert!(parse(&["--icon", "x", "--toggle", ":A:a"]).is_err());
    }

    fn separators(entries: Vec<Entry>) -> usize {
        let tray = Trayler {
            icon: String::new(),
            title: String::new(),
            entries,
            exit_label: "Exit".into(),
            exit_signal: Signal::TERM,
            quit: mpsc::unbounded_channel().0,
            pidfd: None,
        };
        ksni::Tray::menu(&tray)
            .iter()
            .filter(|item| matches!(item, MenuItem::Separator))
            .count()
    }

    #[test]
    fn separates_entries_from_exit_once() {
        assert_eq!(separators(vec![]), 0);
        assert_eq!(separators(vec![button("B", "", "b")]), 1);
        assert_eq!(
            separators(vec![button("B", "", "b"), Entry::Separator {}]),
            1
        );
    }
}
