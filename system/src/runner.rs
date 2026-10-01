//! Command execution abstraction so backends can be tested with canned output.

use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};

use anyhow::{anyhow, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

pub trait CommandRunner: Send + Sync {
    /// Run a command and capture its output (never inherits the shell's stdin).
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;

    /// Run with stdin content.
    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<Output>;

    fn run_ok(&self, program: &str, args: &[&str]) -> Result<String> {
        let out = self.run(program, args)?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            Err(anyhow!(
                "{program} {}: {}",
                args.join(" "),
                if out.stderr.trim().is_empty() {
                    out.stdout.trim()
                } else {
                    out.stderr.trim()
                }
            ))
        }
    }
}

/// Runs real processes with a timeout.
pub struct RealRunner {
    pub timeout: Duration,
}

impl Default for RealRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
        }
    }
}

impl RealRunner {
    fn execute(&self, program: &str, args: &[&str], stdin: Option<&str>) -> Result<Output> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("LC_ALL", "C");
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("spawning {program}: {e}"))?;
        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            use std::io::Write;
            let _ = pipe.write_all(data.as_bytes());
        }
        // Read the pipes on helper threads so a chatty child cannot block on a full pipe while
        // we wait for it.
        let out_thread = drain(child.stdout.take());
        let err_thread = drain(child.stderr.take());
        // The deadline is enforced here rather than with GNU `timeout`, which RustOS's
        // coreutils do not ship.
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status.code().unwrap_or(-1);
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break 124;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = out_thread.join().unwrap_or_default();
        let mut stderr = err_thread.join().unwrap_or_default();
        if status == 124 && stderr.is_empty() {
            stderr = format!("{program} timed out after {:?}", self.timeout).into_bytes();
        }
        Ok(Output {
            status,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

fn drain<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        self.execute(program, args, None)
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<Output> {
        self.execute(program, args, Some(stdin))
    }
}

/// Canned responses keyed by `program arg1 arg2 …` (exact) or `program` (fallback).
#[derive(Default)]
pub struct FakeRunner {
    responses: Mutex<HashMap<String, Output>>,
    pub calls: Mutex<Vec<String>>,
}

impl FakeRunner {
    pub fn with(mut self, key: &str, stdout: &str) -> Self {
        self.responses.get_mut().unwrap().insert(
            key.to_string(),
            Output {
                status: 0,
                stdout: stdout.to_string(),
                stderr: String::new(),
            },
        );
        self
    }

    pub fn with_status(mut self, key: &str, status: i32, stdout: &str) -> Self {
        self.responses.get_mut().unwrap().insert(
            key.to_string(),
            Output {
                status,
                stdout: stdout.to_string(),
                stderr: String::new(),
            },
        );
        self
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        let key = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.lock().unwrap().push(key.clone());
        let map = self.responses.lock().unwrap();
        Ok(map
            .get(&key)
            .or_else(|| map.get(program))
            .cloned()
            .unwrap_or(Output {
                status: 127,
                stdout: String::new(),
                stderr: format!("fake: no response for `{key}`"),
            }))
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &str) -> Result<Output> {
        self.run(program, args)
    }
}

#[cfg(test)]
mod real_runner_tests {
    use super::*;

    #[test]
    fn captures_output_and_enforces_the_deadline() {
        let r = RealRunner::default();
        let out = r
            .run("sh", &["-c", "echo out; echo err >&2; exit 3"])
            .unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout, "out\n");
        assert_eq!(out.stderr, "err\n");
        let out = r.run_with_stdin("cat", &[], "piped").unwrap();
        assert_eq!(out.stdout, "piped");
        let slow = RealRunner {
            timeout: Duration::from_millis(300),
        };
        let start = std::time::Instant::now();
        let out = slow.run("sleep", &["5"]).unwrap();
        assert_eq!(out.status, 124);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(r.run("definitely-not-a-program-edex", &[]).is_err());
    }
}
