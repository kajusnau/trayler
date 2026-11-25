use std::io::Write;
use std::path::Path;

use serde::Deserialize;

use crate::cli::{Cli, ExitSignal};

pub(crate) const EXAMPLE: &str = include_str!("../example.toml");

pub(crate) fn init(path: &Path) -> Result<(), String> {
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
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
    id: Option<String>,
    label: Option<String>,
    signal: Option<ExitSignal>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Entry {
    Button {
        #[serde(default)]
        id: Option<String>,
        label: String,
        #[serde(default)]
        icon: String,
        #[serde(default)]
        command: String,
        #[serde(default)]
        exit: bool,
    },
    Toggle {
        #[serde(default)]
        id: Option<String>,
        label: String,
        #[serde(default)]
        icon: String,
        command: String,
        #[serde(default)]
        checked: bool,
    },
    Separator {},
}

impl Entry {
    pub(crate) fn id(&self) -> Option<&str> {
        match self {
            Entry::Button { id, .. } | Entry::Toggle { id, .. } => id.as_deref(),
            Entry::Separator {} => None,
        }
    }
}

// serde cannot default a missing enum tag, so each entry is read as a table first.
fn entries<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<Entry>, D::Error> {
    use serde::de::Error;
    Vec::<toml::Table>::deserialize(deserializer)?
        .into_iter()
        .enumerate()
        .map(|(index, table)| {
            let kind = table
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("button")
                .to_owned();
            parse_entry(table).map_err(|message| {
                D::Error::custom(format!("entry {} ({kind}): {message}", index + 1))
            })
        })
        .collect()
}

pub(crate) fn parse_entry(mut table: toml::Table) -> Result<Entry, String> {
    table.entry("type").or_insert_with(|| "button".into());
    let entry: Entry = toml::Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| e.message().to_owned())?;
    match &entry {
        Entry::Button { label, .. } | Entry::Toggle { label, .. } if label.is_empty() => {
            Err("label is empty".into())
        }
        Entry::Button {
            command,
            exit: false,
            ..
        }
        | Entry::Toggle { command, .. }
            if command.is_empty() =>
        {
            Err("command is empty".into())
        }
        _ => Ok(entry),
    }
}

pub(crate) fn validate_entries(entries: &[Entry], exit_id: &str) -> Result<(), String> {
    let mut seen = std::collections::HashSet::from([exit_id]);
    for entry in entries {
        if let Entry::Button {
            id: None,
            exit: true,
            label,
            ..
        } = entry
        {
            return Err(format!("\"{label}\": exit = true needs an id"));
        }
        if let Some(id) = entry.id() {
            if id.is_empty() {
                return Err("id is empty".into());
            }
            if !seen.insert(id) {
                return Err(format!("duplicate id \"{id}\""));
            }
        }
    }
    Ok(())
}

pub(crate) fn load_config(name: &str, text: &str) -> Result<Config, String> {
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

pub(crate) fn read_config(path: &Path) -> Result<Config, String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| format!("stdin: {e}"))?;
        return load_config("stdin", &text);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    load_config(&path.display().to_string(), &text)
}

pub(crate) struct Settings {
    pub(crate) icon: String,
    pub(crate) title: String,
    pub(crate) entries: Vec<Entry>,
    pub(crate) exit_label: String,
    pub(crate) exit_signal: ExitSignal,
    pub(crate) exit_id: String,
}

pub(crate) fn settings(cli: &Cli, config: Config) -> Result<Settings, String> {
    let file = match &cli.config {
        Some(p) if p == Path::new("-") => "stdin".into(),
        Some(p) => p.display().to_string(),
        None => String::new(),
    };
    if config.exit.signal.is_some() && cli.pid.is_none() && cli.command.is_empty() {
        return Err(format!("{file}: exit.signal needs --pid or a command"));
    }
    let exit_id = config.exit.id.unwrap_or_else(|| "exit".into());
    validate_entries(&config.entries, &exit_id).map_err(|e| format!("{file}: {e}"))?;
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
        exit_id,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::cli::parse;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("trayler-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("trayler.toml")
    }

    #[test]
    fn the_example_is_a_valid_config() {
        let config = load_config("example.toml", EXAMPLE).unwrap();
        assert_eq!(config.entries.len(), 4);
        assert!(matches!(config.entries[1], Entry::Separator {}));
        assert!(matches!(
            config.entries[2],
            Entry::Toggle { checked: false, .. }
        ));
        assert!(matches!(
            config.entries[3],
            Entry::Button { exit: true, .. }
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

    fn config(text: &str) -> Result<Config, String> {
        load_config("test.toml", text)
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

    #[test]
    fn an_entry_without_type_is_a_button() {
        let config = config("[[entry]]\nlabel = \"Hi\"\ncommand = \"echo hi\"\n").unwrap();
        assert_eq!(config.entries, [button("Hi", "", "echo hi")]);
    }

    #[test]
    fn reads_ids_and_exit_buttons() {
        let config = config(
            "[exit]\nid = \"stop\"\n\
             [[entry]]\nid = \"discard\"\nlabel = \"Discard\"\nexit = true\n",
        )
        .unwrap();
        assert_eq!(config.exit.id.as_deref(), Some("stop"));
        assert_eq!(
            config.entries,
            [Entry::Button {
                id: Some("discard".into()),
                label: "Discard".into(),
                icon: String::new(),
                command: String::new(),
                exit: true,
            }]
        );
    }

    #[test]
    fn validates_ids_and_exit_buttons() {
        let settings_of =
            |text: &str| settings(&parse(&["--icon", "x"]).unwrap(), config(text).unwrap());
        for text in [
            "[[entry]]\nid = \"a\"\nlabel = \"A\"\ncommand = \"a\"\n[[entry]]\nid = \"a\"\nlabel = \"B\"\ncommand = \"b\"\n",
            "[[entry]]\nid = \"exit\"\nlabel = \"A\"\ncommand = \"a\"\n",
            "[[entry]]\nlabel = \"A\"\nexit = true\n",
            "[[entry]]\nid = \"\"\nlabel = \"A\"\ncommand = \"a\"\n",
        ] {
            assert!(settings_of(text).is_err(), "accepted: {text}");
        }
        assert_eq!(
            settings_of(
                "[exit]\nid = \"stop\"\n[[entry]]\nid = \"exit\"\nlabel = \"A\"\nexit = true\n",
            )
            .unwrap()
            .exit_id,
            "stop"
        );
        assert_eq!(settings_of("").unwrap().exit_id, "exit");
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
                    id: None,
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
            "[[entry]]\ntype = \"toggle\"\nid = \"t\"\nlabel = \"T\"\ncommand = \"t\"\nexit = true\n",
            "[[entry]]\ntype = \"separator\"\nid = \"s\"\n",
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
}
