#![cfg_attr(not(test), allow(dead_code))]

use crate::config::{parse_entry, validate_entries, Entry};
use crate::tray::Trayler;

pub(crate) enum Command {
    Icon(String),
    Title(String),
    Set {
        id: String,
        pairs: Vec<(String, String)>,
    },
    Add {
        position: Position,
        pairs: Vec<(String, String)>,
    },
    Remove(String),
}

pub(crate) enum Position {
    End,
    After(String),
    Before(String),
}

fn words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars();
    let mut word: Option<String> = None;
    while let Some(c) = chars.next() {
        match c {
            ' ' => words.extend(word.take()),
            '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => word.push(c),
                            _ => return Err("only \\\" and \\\\ can be escaped".into()),
                        },
                        Some(c) => word.push(c),
                        None => return Err("unclosed quote".into()),
                    }
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

pub(crate) fn quote(word: &str) -> String {
    if !word.is_empty() && !word.contains([' ', '"', '\\']) {
        return word.into();
    }
    format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
}

fn pairs(words: &[String]) -> Result<Vec<(String, String)>, String> {
    words
        .iter()
        .map(|word| match word.split_once('=') {
            Some((key, value)) => Ok((key.into(), value.into())),
            None => Err(format!("expected key=value, got \"{word}\"")),
        })
        .collect()
}

pub(crate) fn parse(line: &str) -> Result<Command, String> {
    let words = words(line)?;
    let (name, args) = words.split_first().ok_or("empty command")?;
    match (name.as_str(), args) {
        ("icon", [name]) => Ok(Command::Icon(name.clone())),
        ("title", [text]) => Ok(Command::Title(text.clone())),
        ("remove", [id]) => Ok(Command::Remove(id.clone())),
        ("set", [id, rest @ ..]) if !rest.is_empty() => Ok(Command::Set {
            id: id.clone(),
            pairs: pairs(rest)?,
        }),
        ("add", rest) => {
            let mut position = Position::End;
            let mut kept = Vec::new();
            for (key, value) in pairs(rest)? {
                match (key.as_str(), &position) {
                    ("after", Position::End) => position = Position::After(value),
                    ("before", Position::End) => position = Position::Before(value),
                    ("after" | "before", _) => return Err("give after or before once".into()),
                    _ => kept.push((key, value)),
                }
            }
            Ok(Command::Add {
                position,
                pairs: kept,
            })
        }
        ("icon" | "title" | "remove" | "set", _) => Err(format!("wrong arguments for {name}")),
        _ => Err(format!("unknown command \"{name}\"")),
    }
}

fn value(key: &str, raw: String) -> Result<toml::Value, String> {
    match (key, raw.as_str()) {
        ("checked" | "exit", "true") => Ok(true.into()),
        ("checked" | "exit", "false") => Ok(false.into()),
        ("checked" | "exit", _) => Err(format!("{key} must be true or false")),
        _ => Ok(raw.into()),
    }
}

fn table(pairs: Vec<(String, String)>, mut table: toml::Table) -> Result<toml::Table, String> {
    for (key, raw) in pairs {
        let value = value(&key, raw)?;
        table.insert(key, value);
    }
    Ok(table)
}

fn index(entries: &[Entry], id: &str) -> Result<usize, String> {
    entries
        .iter()
        .position(|e| e.id() == Some(id))
        .ok_or(format!("no entry \"{id}\""))
}

pub(crate) fn apply(tray: &mut Trayler, command: Command) -> Result<(), String> {
    let mut entries = tray.entries.clone();
    match command {
        Command::Icon(icon) => tray.icon = icon,
        Command::Title(title) => tray.title = title,
        Command::Set { id, pairs } if id == tray.exit_id => match pairs.as_slice() {
            [(key, label)] if key == "label" => tray.exit_label = label.clone(),
            _ => return Err("only the label of the exit entry can be set".into()),
        },
        Command::Set { id, pairs } => {
            if let Some((key, _)) = pairs.iter().find(|(key, _)| {
                ![
                    "label",
                    "icon",
                    "checked",
                    "label_checked",
                    "icon_checked",
                    "command",
                ]
                .contains(&key.as_str())
            }) {
                return Err(format!("{key} cannot be set"));
            }
            let at = index(&entries, &id)?;
            let current = toml::Table::try_from(&entries[at]).map_err(|e| e.to_string())?;
            entries[at] = parse_entry(table(pairs, current)?)?;
        }
        Command::Add { position, pairs } => {
            let entry = parse_entry(table(pairs, toml::Table::new())?)?;
            if entry.id().is_none() {
                return Err("add needs an id".into());
            }
            let at = match position {
                Position::End => entries.len(),
                Position::Before(id) if id == tray.exit_id => entries.len(),
                Position::After(id) if id == tray.exit_id => {
                    return Err("nothing goes after the exit entry".into())
                }
                Position::Before(id) => index(&entries, &id)?,
                Position::After(id) => index(&entries, &id)? + 1,
            };
            entries.insert(at, entry);
        }
        Command::Remove(id) if id == tray.exit_id => {
            return Err("the exit entry cannot be removed".into())
        }
        Command::Remove(id) => {
            entries.remove(index(&entries, &id)?);
        }
    }
    validate_entries(&entries, &tray.exit_id)?;
    tray.entries = entries;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rustix::process::Signal;
    use tokio::sync::mpsc;

    use super::*;

    fn words_of(line: &str) -> Vec<String> {
        words(line).unwrap()
    }

    #[test]
    fn splits_and_unquotes_words() {
        assert_eq!(words_of("set a  label=x"), ["set", "a", "label=x"]);
        assert_eq!(words_of(r#"set a label="x y""#), ["set", "a", "label=x y"]);
        assert_eq!(
            words_of(r#"title "say \"hi\" \\""#),
            ["title", r#"say "hi" \"#]
        );
        assert_eq!(words_of(r#"title """#), ["title", ""]);
        assert!(words(r#"title "open"#).is_err());
        assert!(words(r#"title "\n""#).is_err());
    }

    #[test]
    fn quote_round_trips() {
        for word in ["plain", "two words", r#"q"uo\te"#, ""] {
            assert_eq!(words_of(&quote(word)), [word]);
        }
    }

    #[test]
    fn parses_commands() {
        assert!(matches!(parse("icon x"), Ok(Command::Icon(i)) if i == "x"));
        assert!(matches!(parse("title \"A b\""), Ok(Command::Title(t)) if t == "A b"));
        assert!(matches!(parse("remove a"), Ok(Command::Remove(i)) if i == "a"));
        assert!(matches!(
            parse("add before=a id=b label=B command=b"),
            Ok(Command::Add { position: Position::Before(p), pairs }) if p == "a" && pairs.len() == 3
        ));
        assert!(matches!(
            parse("set a checked=true"),
            Ok(Command::Set { id, pairs }) if id == "a" && pairs == [("checked".into(), "true".into())]
        ));
        for bad in [
            "",
            "bogus",
            "icon",
            "icon a b",
            "set a",
            "set a label",
            "remove",
            "add after=a before=b id=c",
        ] {
            assert!(parse(bad).is_err(), "accepted: {bad}");
        }
    }

    fn button(id: &str) -> Entry {
        let mut table = toml::Table::new();
        for (key, value) in [("id", id), ("label", "L"), ("command", "c")] {
            table.insert(key.into(), value.into());
        }
        parse_entry(table).unwrap()
    }

    fn tray(entries: Vec<Entry>) -> Trayler {
        Trayler {
            icon: "i".into(),
            title: "t".into(),
            entries,
            exit_label: "Exit".into(),
            exit_id: "exit".into(),
            exit_signal: Signal::TERM,
            quit: mpsc::unbounded_channel().0,
            pidfd: None,
        }
    }

    fn run(tray: &mut Trayler, line: &str) -> Result<(), String> {
        apply(tray, parse(line).unwrap())
    }

    fn ids(tray: &Trayler) -> Vec<&str> {
        tray.entries.iter().filter_map(Entry::id).collect()
    }

    #[test]
    fn sets_icon_title_and_entry_keys() {
        let mut tray = tray(vec![button("a")]);
        run(&mut tray, "icon j").unwrap();
        run(&mut tray, "title \"T 2\"").unwrap();
        run(&mut tray, "set a label=M icon=k").unwrap();
        run(&mut tray, "set exit label=Stop").unwrap();
        assert_eq!(
            (
                tray.icon.as_str(),
                tray.title.as_str(),
                tray.exit_label.as_str()
            ),
            ("j", "T 2", "Stop")
        );
        assert!(
            matches!(&tray.entries[0], Entry::Button { label, icon, .. } if label == "M" && icon == "k")
        );
    }

    #[test]
    fn adds_and_removes_entries() {
        let mut tray = tray(vec![button("a")]);
        run(&mut tray, "add id=z label=Z command=z").unwrap();
        run(&mut tray, "add before=a id=b label=B command=b").unwrap();
        run(
            &mut tray,
            "add after=a id=c type=toggle label=C command=c checked=true",
        )
        .unwrap();
        run(&mut tray, "add before=exit id=d type=label label=D").unwrap();
        assert_eq!(ids(&tray), ["b", "a", "c", "z", "d"]);
        run(&mut tray, "remove a").unwrap();
        assert_eq!(ids(&tray), ["b", "c", "z", "d"]);
    }

    #[test]
    fn rejected_commands_change_nothing() {
        let mut tray = tray(vec![button("a")]);
        for line in [
            "set nope label=x",
            "set a nope=x",
            "set a id=b",
            "set a label=",
            "set a checked=true",
            "set exit icon=x",
            "add label=X command=x",
            "add id=a label=X command=x",
            "add id=exit label=X command=x",
            "add after=exit id=x label=X command=x",
            "add after=nope id=x label=X command=x",
            "add id=t type=toggle label=T command=t exit=true",
            "add id=t type=toggle label=T command=t checked=yes",
            "remove exit",
            "remove nope",
        ] {
            assert!(run(&mut tray, line).is_err(), "accepted: {line}");
        }
        assert_eq!(tray.entries, [button("a")]);
        assert_eq!(tray.exit_label, "Exit");
    }

    #[test]
    fn serialized_entries_parse_back() {
        let separator = parse_entry(toml::Table::try_from(Entry::Separator {}).unwrap());
        assert_eq!(separator, Ok(Entry::Separator {}));
        let label = parse_entry(toml::Table::from_iter([
            ("type".into(), "label".into()),
            ("label".into(), "L".into()),
        ]))
        .unwrap();
        assert_eq!(
            parse_entry(toml::Table::try_from(&label).unwrap()),
            Ok(label)
        );
    }

    #[test]
    fn set_does_not_run_the_toggle_command() {
        let marker = std::env::temp_dir().join(format!("trayler-set-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let toggle = format!(
            "add id=t type=toggle label=T command={}",
            quote(&format!("touch {}", marker.display()))
        );
        let mut tray = tray(vec![]);
        run(&mut tray, &toggle).unwrap();
        run(&mut tray, "set t checked=true").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!marker.exists());
    }
}
