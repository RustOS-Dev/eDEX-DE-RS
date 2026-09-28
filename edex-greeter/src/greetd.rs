//! Blocking greetd client over `$GREETD_SOCK`.

use std::{io::Write, os::unix::net::UnixStream, path::Path};

use anyhow::{anyhow, Context, Result};
use greetd_ipc::{codec::SyncCodec, AuthMessageType, ErrorType, Request, Response};

/// What the greeter should do next after talking to greetd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Show a prompt; `secret` masks input.
    Prompt { message: String, secret: bool },
    /// Informational or error text; no input expected (answer with `None`).
    Info { message: String, error: bool },
    /// Authentication succeeded; call `start_session`.
    Success,
    /// Authentication failed (wrong password etc.); the session was cancelled.
    Failed(String),
}

pub struct Greetd {
    stream: UnixStream,
    in_session: bool,
}

impl Greetd {
    pub fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .with_context(|| format!("connecting to greetd at {}", path.display()))?;
        Ok(Self {
            stream,
            in_session: false,
        })
    }

    pub fn from_env() -> Result<Self> {
        let path =
            std::env::var("GREETD_SOCK").context("GREETD_SOCK is not set (run under greetd)")?;
        Self::connect(Path::new(&path))
    }

    fn roundtrip(&mut self, req: &Request) -> Result<Response> {
        req.write_to(&mut self.stream)
            .map_err(|e| anyhow!("greetd write: {e}"))?;
        self.stream.flush()?;
        Response::read_from(&mut self.stream).map_err(|e| anyhow!("greetd read: {e}"))
    }

    fn map(&mut self, resp: Response) -> Result<Step> {
        Ok(match resp {
            Response::Success => Step::Success,
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret => Step::Prompt {
                    message: auth_message,
                    secret: true,
                },
                AuthMessageType::Visible => Step::Prompt {
                    message: auth_message,
                    secret: false,
                },
                AuthMessageType::Info => Step::Info {
                    message: auth_message,
                    error: false,
                },
                AuthMessageType::Error => Step::Info {
                    message: auth_message,
                    error: true,
                },
            },
            Response::Error {
                error_type,
                description,
            } => {
                self.in_session = false;
                match error_type {
                    ErrorType::AuthError => Step::Failed(if description.is_empty() {
                        "authentication failed".into()
                    } else {
                        description
                    }),
                    ErrorType::Error => return Err(anyhow!("greetd error: {description}")),
                }
            }
        })
    }

    pub fn create_session(&mut self, username: &str) -> Result<Step> {
        if self.in_session {
            let _ = self.cancel();
        }
        let resp = self.roundtrip(&Request::CreateSession {
            username: username.to_string(),
        })?;
        self.in_session = true;
        self.map(resp)
    }

    pub fn respond(&mut self, answer: Option<String>) -> Result<Step> {
        let resp = self.roundtrip(&Request::PostAuthMessageResponse { response: answer })?;
        self.map(resp)
    }

    pub fn start_session(&mut self, cmd: Vec<String>, env: Vec<String>) -> Result<()> {
        match self.roundtrip(&Request::StartSession { cmd, env })? {
            Response::Success => Ok(()),
            Response::Error { description, .. } => Err(anyhow!("start session: {description}")),
            Response::AuthMessage { .. } => Err(anyhow!("unexpected auth message after success")),
        }
    }

    pub fn cancel(&mut self) -> Result<()> {
        self.in_session = false;
        let _ = self.roundtrip(&Request::CancelSession)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A fake greetd that expects the password "hunter2".
    fn fake_greetd(sock: std::path::PathBuf) -> std::thread::JoinHandle<Vec<String>> {
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut log = Vec::new();
            loop {
                let req = match Request::read_from(&mut s) {
                    Ok(r) => r,
                    Err(_) => break,
                };
                log.push(format!("{req:?}"));
                let resp = match req {
                    Request::CreateSession { username } if username == "ari" => {
                        Response::AuthMessage {
                            auth_message_type: AuthMessageType::Secret,
                            auth_message: "Password: ".into(),
                        }
                    }
                    Request::CreateSession { .. } => Response::Error {
                        error_type: ErrorType::AuthError,
                        description: "no such user".into(),
                    },
                    Request::PostAuthMessageResponse { response } => {
                        if response.as_deref() == Some("hunter2") {
                            Response::Success
                        } else {
                            Response::Error {
                                error_type: ErrorType::AuthError,
                                description: "bad password".into(),
                            }
                        }
                    }
                    Request::StartSession { cmd, .. } => {
                        assert_eq!(cmd, vec!["edex-session"]);
                        Response::Success
                    }
                    Request::CancelSession => Response::Success,
                };
                resp.write_to(&mut s).unwrap();
                if matches!(log.last().map(|s| s.as_str()), Some(l) if l.starts_with("StartSession"))
                {
                    break;
                }
            }
            log
        })
    }

    #[test]
    fn full_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("greetd.sock");
        let server = fake_greetd(sock.clone());
        let mut g = Greetd::connect(&sock).unwrap();
        assert_eq!(
            g.create_session("ari").unwrap(),
            Step::Prompt {
                message: "Password: ".into(),
                secret: true
            }
        );
        assert!(matches!(
            g.respond(Some("wrong".into())).unwrap(),
            Step::Failed(_)
        ));
        assert_eq!(
            g.create_session("ari").unwrap(),
            Step::Prompt {
                message: "Password: ".into(),
                secret: true
            }
        );
        assert_eq!(g.respond(Some("hunter2".into())).unwrap(), Step::Success);
        g.start_session(
            vec!["edex-session".into()],
            vec!["XDG_SESSION_TYPE=wayland".into()],
        )
        .unwrap();
        let log = server.join().unwrap();
        assert!(log.iter().any(|l| l.starts_with("StartSession")));
    }

    #[test]
    fn unknown_user_fails() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("greetd.sock");
        let _server = fake_greetd(sock.clone());
        let mut g = Greetd::connect(&sock).unwrap();
        assert!(matches!(
            g.create_session("nobody-here").unwrap(),
            Step::Failed(_)
        ));
    }
}
