//! Minimal arg parser: --key value / --key=value / --bool / -abc shorthand
//! booleans, plus positionals. Mirrors the original Go parser.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Flags {
    pub values: HashMap<String, String>,
    pub bools: HashMap<String, bool>,
    pub positionals: Vec<String>,
}

impl Flags {
    pub fn parse(args: &[String]) -> Flags {
        let mut flags = Flags::default();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-" || arg == "--" {
                flags.positionals.push(arg.clone());
                i += 1;
                continue;
            }
            if arg.starts_with('-') && !arg.starts_with("--") {
                for ch in arg.trim_start_matches('-').chars() {
                    flags.bools.insert(ch.to_string(), true);
                }
                i += 1;
                continue;
            }
            if !arg.starts_with("--") {
                flags.positionals.push(arg.clone());
                i += 1;
                continue;
            }
            let name_value = arg.trim_start_matches("--");
            if name_value.is_empty() {
                i += 1;
                continue;
            }
            if let Some((name, value)) = name_value.split_once('=') {
                flags.values.insert(name.to_string(), value.to_string());
                flags.bools.insert(name.to_string(), true);
                i += 1;
                continue;
            }
            if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags
                    .values
                    .insert(name_value.to_string(), args[i + 1].clone());
                flags.bools.insert(name_value.to_string(), true);
                i += 2;
                continue;
            }
            flags.bools.insert(name_value.to_string(), true);
            i += 1;
        }
        if flags.bool("json") && flags.value("format").is_empty() {
            flags.values.insert("format".into(), "json".into());
        }
        flags
    }

    /// Value for a flag, or "" if absent.
    pub fn value(&self, key: &str) -> &str {
        self.values.get(key).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn bool(&self, key: &str) -> bool {
        self.bools.get(key).copied().unwrap_or(false)
    }

    pub fn wants_json(&self) -> bool {
        self.value("format").eq_ignore_ascii_case("json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Flags {
        Flags::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_values_bools_positionals() {
        let f = v(&["search", "playwright", "--stack", "next", "--json"]);
        assert_eq!(f.positionals, vec!["search", "playwright"]);
        assert_eq!(f.value("stack"), "next");
        assert!(f.wants_json());
    }

    #[test]
    fn equals_form_and_yes() {
        let f = v(&["install", "owner/x", "--surface=.claude/skills", "--yes"]);
        assert_eq!(f.value("surface"), ".claude/skills");
        assert!(f.bool("yes"));
    }
}
