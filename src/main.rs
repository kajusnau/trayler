mod cli;
mod config;
mod process;
mod tray;

use std::io::Write;
use std::sync::Arc;

use clap::CommandFactory;
use ksni::TrayMethods;
use rustix::process::{pidfd_send_signal, Signal};
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

use cli::{parse_cli, Action, Cli};
use config::{init, read_config, settings, Config};
use process::{exit_code, open_pidfd};
use tray::Trayler;

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
