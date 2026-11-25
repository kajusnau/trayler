use std::os::fd::OwnedFd;
use std::process::Command;
use std::sync::Arc;

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip};
use rustix::process::{pidfd_send_signal, Signal};
use tokio::sync::mpsc;

use crate::config::Entry;

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

pub(crate) struct Trayler {
    pub(crate) icon: String,
    pub(crate) title: String,
    pub(crate) entries: Vec<Entry>,
    pub(crate) exit_label: String,
    pub(crate) exit_id: String,
    pub(crate) exit_signal: Signal,
    pub(crate) quit: mpsc::UnboundedSender<String>,
    pub(crate) pidfd: Option<Arc<OwnedFd>>,
}

impl Trayler {
    // Queue the id before signalling, so it is there by the time main sees the process exit.
    fn exit(&self, id: &str) {
        let _ = self.quit.send(id.to_owned());
        if let Some(pidfd) = &self.pidfd {
            if let Err(e) = pidfd_send_signal(pidfd, self.exit_signal) {
                eprintln!("trayler: {e}");
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
                    id,
                    label,
                    icon,
                    command,
                    exit,
                } => {
                    let (id, command, exit) = (id.clone(), command.clone(), *exit);
                    StandardItem {
                        label: menu_label(label),
                        icon_name: icon.clone(),
                        activate: Box::new(move |this: &mut Self| {
                            if !command.is_empty() {
                                spawn_command(&command, None);
                            }
                            if exit {
                                // validate_entries guarantees exit buttons have an id.
                                this.exit(id.as_deref().unwrap_or_default());
                            }
                        }),
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
                activate: Box::new(|this: &mut Self| this.exit(&this.exit_id.clone())),
                ..Default::default()
            }
            .into(),
        );
        items
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

    fn button(label: &str, icon: &str, command: &str) -> Entry {
        Entry::Button {
            id: None,
            label: label.into(),
            icon: icon.into(),
            command: command.into(),
            exit: false,
        }
    }

    fn tray(
        entries: Vec<Entry>,
        pidfd: Option<Arc<OwnedFd>>,
    ) -> (Trayler, mpsc::UnboundedReceiver<String>) {
        let (quit, quit_rx) = mpsc::unbounded_channel();
        let tray = Trayler {
            icon: String::new(),
            title: String::new(),
            entries,
            exit_label: "Exit".into(),
            exit_id: "exit".into(),
            exit_signal: Signal::TERM,
            quit,
            pidfd,
        };
        (tray, quit_rx)
    }

    fn activate(tray: &mut Trayler, index: usize) {
        match ksni::Tray::menu(tray).swap_remove(index) {
            MenuItem::Standard(item) => (item.activate)(tray),
            MenuItem::Checkmark(item) => (item.activate)(tray),
            _ => panic!("item {index} cannot be activated"),
        }
    }

    fn separators(entries: Vec<Entry>) -> usize {
        let (tray, _) = tray(entries, None);
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

    #[test]
    fn exit_entries_report_their_id() {
        let discard = Entry::Button {
            id: Some("discard".into()),
            label: "Discard".into(),
            icon: String::new(),
            command: String::new(),
            exit: true,
        };
        let (mut tray, mut quit_rx) = tray(vec![discard], None);
        activate(&mut tray, 0);
        assert_eq!(quit_rx.try_recv().unwrap(), "discard");
        activate(&mut tray, 2);
        assert_eq!(quit_rx.try_recv().unwrap(), "exit");
    }

    #[test]
    fn exit_entries_signal_the_followed_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pidfd = Arc::new(crate::process::open_pidfd(child.id() as i32).unwrap());
        let (mut tray, mut quit_rx) = tray(vec![], Some(pidfd));
        activate(&mut tray, 0);
        assert_eq!(quit_rx.try_recv().unwrap(), "exit");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(15));
    }
}
