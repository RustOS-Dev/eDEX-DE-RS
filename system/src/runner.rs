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
        cmd.args(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        cmd.env("LC_ALL", "C");
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("spawning {program}: {e}"))?;
        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            use std::io::Write;
            let _ = pipe.write_all(data.as_bytes());
        }
        // Read both pipes on threads so a full pipe never blocks the child, and give up
        // (killing it) after the timeout.
        let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut buf);
                }
                buf
            })
        };
        let out = read(child.stdout.take().map(|p| Box::new(p) as _));
        let err = read(child.stderr.take().map(|p| Box::new(p) as _));
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait()? {
                Some(st) => break st.code().unwrap_or(-1),
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break 124;
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        Ok(Output {
            status,
            stdout: String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned(),
        })
    }
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
