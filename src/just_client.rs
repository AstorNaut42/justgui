// Talks to the `just` CLI: introspects a justfile via `just --dump
// --dump-format json` and builds the shell command to run a recipe.
// `just` itself is the parser -- we never read justfile syntax ourselves.
use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct JustParam {
    pub name: String,
    pub default_value: String, // only meaningful if has_default
    pub has_default: bool,
    pub variadic: bool, // `*args` / `+args`: zero-or-more / one-or-more
}

#[derive(Debug, Clone, Default)]
pub struct JustRecipe {
    pub name: String,
    pub doc: String,
    pub params: Vec<JustParam>,
    pub is_private: bool,
}

#[derive(Debug, Clone, Default)]
pub struct JustModel {
    pub justfile_path: String,
    pub recipes: Vec<JustRecipe>,
    pub error: String,
}

#[derive(serde::Deserialize)]
struct DumpRoot {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    recipes: BTreeMap<String, DumpRecipe>,
}

#[derive(serde::Deserialize)]
struct DumpRecipe {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    doc: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    parameters: Vec<DumpParam>,
}

#[derive(serde::Deserialize)]
struct DumpParam {
    #[serde(default)]
    name: String,
    #[serde(default)]
    default: Option<serde_json::Value>,
    #[serde(default)]
    kind: String,
}

/// Runs `just --dump --dump-format json` in `dir` and parses the result.
/// Blocks briefly; intended for use on load/reload, not every frame.
pub fn load_justfile(dir: &str) -> JustModel {
    let mut model = JustModel::default();

    let output = Command::new("just")
        .args(["--dump", "--dump-format", "json"])
        .current_dir(dir)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => {
            model.error = "failed to launch `just`".to_string();
            return model;
        }
    };

    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        model.error = if text.is_empty() {
            "`just` exited with an error".to_string()
        } else {
            text
        };
        return model;
    }

    let root: DumpRoot = match serde_json::from_slice(&output.stdout) {
        Ok(r) => r,
        Err(e) => {
            model.error = format!("could not parse `just` output: {e}");
            return model;
        }
    };

    if let Some(src) = root.source {
        model.justfile_path = src;
    }

    let mut recipes: Vec<JustRecipe> = root
        .recipes
        .into_iter()
        .map(|(key, r)| {
            let params = r
                .parameters
                .into_iter()
                .map(|p| {
                    let (has_default, default_value) = match p.default {
                        Some(serde_json::Value::String(s)) => (true, s),
                        _ => (false, String::new()),
                    };
                    JustParam {
                        name: p.name,
                        default_value,
                        has_default,
                        variadic: p.kind == "star" || p.kind == "plus",
                    }
                })
                .collect();
            JustRecipe {
                name: r.name.unwrap_or(key),
                doc: r.doc.unwrap_or_default(),
                params,
                is_private: r.private,
            }
        })
        .collect();

    recipes.sort_by(|a, b| a.name.cmp(&b.name));
    model.recipes = recipes;
    model
}

/// Validates `buffer` (typically the *unsaved* editor content, not what's
/// on disk) by piping it through `just --justfile - --dump --dump-format
/// json` in `dir` -- reuses the same "just is the parser" approach
/// `load_justfile` relies on. Returns an empty string if it parses
/// cleanly, or `just`'s error message otherwise. Never fatal: any failure
/// to even launch `just` just reports no error, rather than blocking
/// editing over a linting hiccup.
pub fn lint_justfile(buffer: &str, dir: &str) -> String {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = match Command::new("just")
        .args(["--justfile", "-", "--dump", "--dump-format", "json"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(buffer.as_bytes());
    } // dropped here, sending EOF

    match child.wait_with_output() {
        Ok(o) if !o.status.success() => String::from_utf8_lossy(&o.stderr).trim().to_string(),
        _ => String::new(),
    }
}

/// Quotes `arg` so it survives being passed through `/bin/sh -c` (POSIX)
/// or `cmd.exe /c` (Windows) as a single token.
pub fn shell_quote(arg: &str) -> String {
    if cfg!(windows) {
        let mut out = String::from("\"");
        for c in arg.chars() {
            if c == '"' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    } else {
        let mut out = String::from("'");
        for c in arg.chars() {
            if c == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(c);
            }
        }
        out.push('\'');
        out
    }
}

/// Builds a `just <recipe> <args...>` command line, shell-quoting each
/// argument. `param_values[i]` corresponds to `recipe.params[i]`; variadic
/// parameters are split on whitespace into multiple CLI arguments.
pub fn build_run_command(recipe: &JustRecipe, param_values: &[String]) -> String {
    let mut cmd = format!("just {}", shell_quote(&recipe.name));

    let n = param_values.len().min(recipe.params.len());

    // Recipe arguments are positional, so a default-valued param can only
    // be omitted if every param after it is also left at its default.
    // Find the last param that actually needs to be passed on the CLI.
    let mut last_needed = 0usize;
    let mut any_needed = false;
    for i in 0..n {
        let param = &recipe.params[i];
        let overridden = if param.variadic {
            !param_values[i].is_empty()
        } else {
            !param.has_default || param_values[i] != param.default_value
        };
        if overridden {
            last_needed = i;
            any_needed = true;
        }
    }
    if !any_needed {
        return cmd;
    }

    for i in 0..=last_needed {
        let param = &recipe.params[i];
        let value = &param_values[i];
        if param.variadic {
            for tok in value.split_whitespace() {
                cmd.push(' ');
                cmd.push_str(&shell_quote(tok));
            }
        } else {
            let v = if value.is_empty() && param.has_default {
                &param.default_value
            } else {
                value
            };
            cmd.push(' ');
            cmd.push_str(&shell_quote(v));
        }
    }
    cmd
}

#[cfg(test)]
mod lint_tests {
    use super::*;

    #[test]
    fn lint_justfile_accepts_valid_syntax() {
        assert_eq!(lint_justfile("build:\n    echo hi\n", "."), "");
    }

    #[test]
    fn lint_justfile_reports_invalid_syntax() {
        let error = lint_justfile("this is not a justfile\n", ".");
        assert!(!error.is_empty(), "expected a lint error for invalid syntax");
    }
}
