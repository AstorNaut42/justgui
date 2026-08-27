// Normalizes a raw pty output chunk into the flat, append-only `run_log`
// buffer. Deliberately not a terminal emulator: no color, no absolute
// cursor addressing (Slint's TextEdit only holds plain text, there's
// nowhere to put either). Handles just what real CLI tools actually rely
// on once they detect a tty: CRLF line endings, `\r`-based progress-bar
// redraws, and ANSI CSI/OSC escape codes, which a naive append would
// otherwise dump as literal garbage into the log.
pub fn append_chunk(log: &mut String, chunk: &str) {
    let mut chars = chunk.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => consume_escape(&mut chars),
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                log.push('\n');
            }
            '\r' => {
                // Bare CR: redraw the current line, matching progress-bar
                // output -- erase back to the last newline and continue.
                let cut = log.rfind('\n').map_or(0, |p| p + 1);
                log.truncate(cut);
            }
            c => log.push(c),
        }
    }
}

/// Best-effort guess that a still-running recipe is blocked waiting on
/// input (a password prompt, a `[y/N]` confirmation, a `select`-menu
/// prompt) rather than just being slow. There's no real terminal-state
/// introspection here (no light dependency offers that) -- just a heuristic
/// over the last, still-unterminated line of output: if it mentions
/// "password"/"passphrase", or ends in `:`, `?`, `>` or `]` (common prompt
/// punctuation), it's probably sitting there waiting on a reply.
pub fn looks_like_prompt(log: &str) -> bool {
    let tail = log.rsplit('\n').next().unwrap_or("").trim_end();
    if tail.is_empty() {
        return false;
    }
    let lower = tail.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("passphrase") {
        return true;
    }
    matches!(tail.chars().last(), Some(':' | '?' | '>' | ']'))
}

fn consume_escape(chars: &mut std::iter::Peekable<std::str::Chars>) {
    match chars.peek() {
        Some('[') => {
            // CSI: ESC [ ... <final byte 0x40..=0x7e>
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            // OSC: ESC ] ... (BEL | ESC \)
            chars.next();
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == '\x07' {
                    break;
                }
                if c == '\x1b' && chars.peek() == Some(&'\\') {
                    chars.next();
                    break;
                }
            }
        }
        _ => {
            chars.next(); // simple two-byte escape (e.g. ESC =, ESC >)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunk: &str) -> String {
        let mut log = String::new();
        append_chunk(&mut log, chunk);
        log
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(run("hello world\n"), "hello world\n");
    }

    #[test]
    fn crlf_collapses_to_lf() {
        assert_eq!(run("a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn bare_cr_redraws_current_line() {
        assert_eq!(run("10%\r50%\r100%\n"), "100%\n");
    }

    #[test]
    fn bare_cr_only_erases_back_to_last_newline() {
        assert_eq!(run("line one\n10%\r100%\n"), "line one\n100%\n");
    }

    #[test]
    fn ansi_csi_color_codes_are_stripped() {
        assert_eq!(run("\x1b[32mHELLO\x1b[0m\n"), "HELLO\n");
    }

    #[test]
    fn ansi_osc_title_sequence_is_stripped_without_eating_following_text() {
        assert_eq!(run("\x1b]0;window title\x07after\n"), "after\n");
    }

    #[test]
    fn recognizes_common_prompt_shapes() {
        assert!(looks_like_prompt("[sudo] password for user: "));
        assert!(looks_like_prompt("Continue? [y/N] "));
        assert!(looks_like_prompt("profile> "));
        assert!(looks_like_prompt("some prior line\nEnter passphrase:"));
    }

    #[test]
    fn does_not_flag_ordinary_output() {
        assert!(!looks_like_prompt("Building project...\n"));
        assert!(!looks_like_prompt(""));
        assert!(!looks_like_prompt("Building project"));
    }
}
