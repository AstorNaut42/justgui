// Reads and writes a project's `.env` file for the Settings popup. This is
// deliberately the *same* file `just` itself will load if the justfile has
// `set dotenv-load := true` -- justgui only edits it, `just` is left to do
// the actual work of getting these values into a recipe's environment, same
// "just is the source of truth" approach the rest of the app takes.
//
// Preserves comments and line order on save: each line is kept verbatim
// unless it's a recognized `KEY=VALUE` assignment, so hand-added comments
// and blank lines in a hand-edited `.env` survive a round trip.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Var { key: String, value: String, exported: bool },
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct EnvFile {
    lines: Vec<Line>,
}

fn env_path(dir: &str) -> PathBuf {
    Path::new(dir).join(".env")
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

fn quote_if_needed(value: &str) -> String {
    let needs_quoting = value.is_empty() || value.chars().any(|c| c.is_whitespace() || c == '#' || c == '"');
    if needs_quoting {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn parse_line(raw: &str) -> Line {
    let trimmed = raw.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Line::Other(raw.to_string());
    }
    let exported = trimmed.starts_with("export ") || trimmed.starts_with("export\t");
    let rest = if exported { trimmed["export".len()..].trim_start() } else { trimmed };
    let Some(eq) = rest.find('=') else {
        return Line::Other(raw.to_string());
    };
    let key = rest[..eq].trim();
    if !is_valid_key(key) {
        return Line::Other(raw.to_string());
    }
    let value = unquote(rest[eq + 1..].trim());
    Line::Var { key: key.to_string(), value, exported }
}

impl EnvFile {
    /// Loads `.env` from `dir`. A missing file is a normal, empty starting
    /// point (most projects don't have one yet) rather than an error.
    pub fn load(dir: &str) -> Self {
        let Ok(text) = std::fs::read_to_string(env_path(dir)) else {
            return Self::default();
        };
        Self { lines: text.lines().map(parse_line).collect() }
    }

    pub fn save(&self, dir: &str) -> std::io::Result<()> {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Var { key, value, exported } => {
                    if *exported {
                        out.push_str("export ");
                    }
                    out.push_str(key);
                    out.push('=');
                    out.push_str(&quote_if_needed(value));
                }
                Line::Other(raw) => out.push_str(raw),
            }
            out.push('\n');
        }
        std::fs::write(env_path(dir), out)
    }

    /// Ordered `(key, value)` pairs, in file order.
    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.lines.iter().filter_map(|l| match l {
            Line::Var { key, value, .. } => Some((key.as_str(), value.as_str())),
            Line::Other(_) => None,
        })
    }

    /// Updates `key`'s value if it exists, otherwise appends a new
    /// unexported `KEY=VALUE` line at the end.
    pub fn set(&mut self, key: &str, value: &str) {
        for line in &mut self.lines {
            if let Line::Var { key: k, value: v, .. } = line {
                if k == key {
                    *v = value.to_string();
                    return;
                }
            }
        }
        self.lines.push(Line::Var { key: key.to_string(), value: value.to_string(), exported: false });
    }

    pub fn remove(&mut self, key: &str) {
        self.lines.retain(|l| !matches!(l, Line::Var { key: k, .. } if k == key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> EnvFile {
        EnvFile { lines: text.lines().map(parse_line).collect() }
    }

    fn render(f: &EnvFile) -> String {
        let dir = std::env::temp_dir().join(format!("justgui-envfile-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_string_lossy().into_owned();
        f.save(&dir).unwrap();
        let text = std::fs::read_to_string(Path::new(&dir).join(".env")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        text
    }

    // Cheap unique-ish suffix so parallel tests don't collide on temp dirs.
    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64
    }

    #[test]
    fn parses_simple_assignments() {
        let f = parse("FOO=bar\nBAZ=1\n");
        assert_eq!(f.vars().collect::<Vec<_>>(), vec![("FOO", "bar"), ("BAZ", "1")]);
    }

    #[test]
    fn strips_matching_quotes() {
        let f = parse("FOO=\"hello world\"\nBAR='single'\n");
        assert_eq!(f.vars().collect::<Vec<_>>(), vec![("FOO", "hello world"), ("BAR", "single")]);
    }

    #[test]
    fn comments_and_blank_lines_are_not_vars() {
        let f = parse("# a comment\n\nFOO=bar\n");
        assert_eq!(f.vars().collect::<Vec<_>>(), vec![("FOO", "bar")]);
    }

    #[test]
    fn export_prefix_is_recognized_and_preserved_on_save() {
        let f = parse("export FOO=bar\n");
        assert_eq!(f.vars().collect::<Vec<_>>(), vec![("FOO", "bar")]);
        assert_eq!(render(&f), "export FOO=bar\n");
    }

    #[test]
    fn set_updates_existing_var_in_place() {
        let mut f = parse("# keep me\nFOO=old\nBAR=1\n");
        f.set("FOO", "new");
        assert_eq!(render(&f), "# keep me\nFOO=new\nBAR=1\n");
    }

    #[test]
    fn set_appends_new_var() {
        let mut f = parse("FOO=1\n");
        f.set("BAR", "2");
        assert_eq!(render(&f), "FOO=1\nBAR=2\n");
    }

    #[test]
    fn set_quotes_values_containing_whitespace() {
        let mut f = EnvFile::default();
        f.set("FOO", "hello world");
        assert_eq!(render(&f), "FOO=\"hello world\"\n");
    }

    #[test]
    fn remove_drops_the_var_but_keeps_other_lines() {
        let mut f = parse("# note\nFOO=1\nBAR=2\n");
        f.remove("FOO");
        assert_eq!(render(&f), "# note\nBAR=2\n");
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = std::env::temp_dir().join(format!("justgui-envfile-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = EnvFile::load(&dir.to_string_lossy());
        assert_eq!(f.vars().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
