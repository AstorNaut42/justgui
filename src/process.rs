// Runs a shell command in the background and streams its combined
// stdout+stderr back to the caller. Backed by `sh -c`/`cmd /C`, so it works
// unmodified on Linux, macOS and Windows without extra dependencies.
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

struct Inner {
    buffer: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
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
    /// `cwd`. Returns false if a process is already running.
    pub fn start(&mut self, command: &str, cwd: &str) -> bool {
        if self.running() {
            return false;
        }
        if let Some(inner) = self.inner.take() {
            if let Some(h) = inner.handle {
                let _ = h.join();
            }
        }

        let full = format!("{command} 2>&1");
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", &full]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", &full]);
            c
        };
        cmd.current_dir(cwd);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut stdout = match child.stdout.take() {
            Some(s) => s,
            None => return false,
        };

        let buffer = Arc::new(Mutex::new(String::new()));
        let running = Arc::new(AtomicBool::new(true));
        let exit_code = Arc::new(AtomicI32::new(0));

        let buf = buffer.clone();
        let run = running.clone();
        let code = exit_code.clone();

        let handle = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        buf.lock().unwrap().push_str(&text);
                    }
                    Err(_) => break,
                }
            }
            let status = child.wait();
            let c = match status {
                Ok(s) => s.code().unwrap_or(-1),
                Err(_) => -1,
            };
            code.store(c, Ordering::SeqCst);
            run.store(false, Ordering::SeqCst);
        });

        self.inner = Some(Inner {
            buffer,
            running,
            exit_code,
            handle: Some(handle),
        });
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
}
