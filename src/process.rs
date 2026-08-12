// Runs a shell command in the background and streams its output back to
// the caller, with the child attached to a real pseudo-terminal (not just
// a plain pipe). A pty is what lets `sudo`, `ssh`, and anything else that
// insists on a controlling terminal actually work through the app's input
// box -- a plain pipe stdin is invisible to those (they open `/dev/tty`
// directly rather than reading a piped stdin). Backed by `portable-pty`,
// which is cross-platform (real ptys on Unix, ConPTY on Windows), so this
// still works unmodified on Linux, macOS and Windows.
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

struct Inner {
    buffer: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    // Kept alive for the pty's lifetime; the reader/writer above are cloned
    // handles that only stay valid while this isn't dropped.
    _master: Box<dyn MasterPty + Send>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub struct Process {
    inner: Option<Inner>,
}

impl Process {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts `command` (a full shell command line) with working directory
    /// `cwd`. Returns false if a process is already running or the pty/
    /// child failed to spawn.
    pub fn start(&mut self, command: &str, cwd: &str) -> bool {
        if self.running() {
            return false;
        }
        if let Some(inner) = self.inner.take() {
            if let Some(h) = inner.handle {
                let _ = h.join();
            }
        }

        let pty_system = native_pty_system();
        // Generously wide so the pty's own hard-wrap rarely fights with the
        // output panel's own word-wrap.
        let pair = match pty_system.openpty(PtySize {
            rows: 50,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let mut cmd = if cfg!(windows) {
            let mut c = CommandBuilder::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = CommandBuilder::new("sh");
            c.args(["-c", command]);
            c
        };
        cmd.cwd(cwd);

        let mut child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(_) => return false,
        };
        drop(pair.slave);

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(_) => return false,
        };
        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(_) => return false,
        };

        let buffer = Arc::new(Mutex::new(String::new()));
        let running = Arc::new(AtomicBool::new(true));
        let exit_code = Arc::new(AtomicI32::new(0));
        let writer = Arc::new(Mutex::new(Some(writer)));

        let buf = buffer.clone();
        let run = running.clone();
        let code = exit_code.clone();
        let writer_for_thread = writer.clone();

        let handle = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        buf.lock().unwrap().push_str(&text);
                    }
                    Err(_) => break,
                }
            }
            let c = match child.wait() {
                Ok(status) => status.exit_code() as i32,
                Err(_) => -1,
            };
            code.store(c, Ordering::SeqCst);
            // Drop any still-open writer now that the child has exited, so
            // a late `send_input` fails cleanly instead of writing into a
            // handle whose process is already gone.
            *writer_for_thread.lock().unwrap() = None;
            run.store(false, Ordering::SeqCst);
        });

        self.inner = Some(Inner {
            buffer,
            running,
            exit_code,
            writer,
            _master: pair.master,
            handle: Some(handle),
        });
        true
    }

    /// Writes `line` followed by a newline to the running child's pty.
    /// Returns false if nothing is running or the write failed (e.g. the
    /// child already exited). The pty's own line discipline echoes this
    /// back through the output stream, same as a real terminal would.
    pub fn send_input(&self, line: &str) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        if !inner.running.load(Ordering::SeqCst) {
            return false;
        }
        let mut guard = inner.writer.lock().unwrap();
        match guard.as_mut() {
            Some(writer) => writer
                .write_all(line.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .is_ok(),
            None => false,
        }
    }

    /// Closes the running child's pty input, sending it EOF. This is the
    /// only way to unstick a recipe that reads stdin to EOF without
    /// prompting (there is no "kill" -- closing input is the escape hatch).
    pub fn close_input(&self) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        *inner.writer.lock().unwrap() = None;
        true
    }

    /// Non-blocking: returns any output collected since the last call, and
    /// whether the process has exited.
    pub fn poll(&self) -> (String, bool) {
        match &self.inner {
            Some(inner) => {
                let mut buf = inner.buffer.lock().unwrap();
                let chunk = std::mem::take(&mut *buf);
                (chunk, !inner.running.load(Ordering::SeqCst))
            }
            None => (String::new(), true),
        }
    }

    pub fn running(&self) -> bool {
        self.inner
            .as_ref()
            .map(|i| i.running.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    pub fn exit_code(&self) -> i32 {
        self.inner
            .as_ref()
            .map(|i| i.exit_code.load(Ordering::SeqCst))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn streams_output_and_exit_code() {
        let mut p = Process::new();
        assert!(p.start("sh -c 'echo hello; echo world 1>&2; exit 3'", "."));

        let mut collected = String::new();
        loop {
            let (chunk, finished) = p.poll();
            collected.push_str(&chunk);
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(collected.contains("hello"));
        assert!(collected.contains("world")); // stderr merged into stdout
        assert_eq!(p.exit_code(), 3);
    }

    #[test]
    fn sends_input_to_running_process() {
        let mut p = Process::new();
        assert!(p.start("sh -c 'read x; echo got:$x'", "."));
        assert!(p.send_input("hi"));

        let mut collected = String::new();
        loop {
            let (chunk, finished) = p.poll();
            collected.push_str(&chunk);
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(collected.contains("got:hi"));
    }

    #[test]
    fn close_input_unblocks_a_read_to_eof() {
        let mut p = Process::new();
        assert!(p.start("sh -c 'cat; echo done'", "."));
        assert!(p.close_input());

        let mut collected = String::new();
        loop {
            let (chunk, finished) = p.poll();
            collected.push_str(&chunk);
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(collected.contains("done"));
    }
}
