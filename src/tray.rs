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
    pub(crate) exit_signal: Signal,
    pub(crate) quit: mpsc::UnboundedSender<()>,
    pub(crate) pidfd: Option<Arc<OwnedFd>>,
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
                    ..
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
