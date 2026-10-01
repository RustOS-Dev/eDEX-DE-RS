//! `edex-auth`: password checks and account edits for eDEX-DE on RustOS, which has no PAM.
//!
//! ```text
//! edex-auth check USER        password on stdin; exit 0 when it is right
//! edex-auth status USER       prints usable | locked | nopassword
//! edex-auth passwd USER       old and new password on stdin (root: the old line may be empty)
//! edex-auth chfn USER NAME    set the full name
//! edex-auth lock USER         root only
//! edex-auth unlock USER       root only
//! ```
//!
//! edex-comp (root) runs `check` for the greeter and the lock screen. The account edits need
//! root too; on a multi-user system install it set-uid root: callers may only change their own
//! account.

use std::io::BufRead;

use anyhow::{bail, Result};
use edex_auth::Accounts;

const FAILURES: &str = "/run/edex-auth";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }
}

fn caller() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

fn stdin_lines(n: usize) -> Vec<String> {
    let stdin = std::io::stdin();
    let mut lines: Vec<String> = stdin.lock().lines().take(n).map_while(Result::ok).collect();
    lines.resize(n, String::new());
    lines
}

/// Root may act on anyone; others only on their own account.
fn allowed(accounts: &Accounts, user: &str) -> Result<()> {
    let me = caller();
    if me == 0 || accounts.uid(user) == Some(me) {
        Ok(())
    } else {
        bail!("permission denied")
    }
}

fn run(args: &[String]) -> Result<()> {
    let accounts = Accounts::default();
    let arg = |i: usize| -> Result<&str> {
        args.get(i).map(String::as_str).ok_or_else(|| {
            anyhow::anyhow!("usage: edex-auth check|status|passwd|chfn|lock|unlock USER …")
        })
    };
    match arg(0)? {
        "check" => {
            let user = arg(1)?;
            let password = stdin_lines(1).remove(0);
            let ok = accounts.check(user, &password);
            let delay = edex_auth::failure_delay(std::path::Path::new(FAILURES), user, !ok);
            if !ok {
                std::thread::sleep(delay.max(std::time::Duration::from_secs(1)));
                bail!("authentication failed");
            }
            Ok(())
        }
        "status" => {
            let user = arg(1)?;
            allowed(&accounts, user)?;
            println!("{}", accounts.status(user)?.as_str());
            Ok(())
        }
        "passwd" => {
            let user = arg(1)?;
            allowed(&accounts, user)?;
            let lines = stdin_lines(2);
            if caller() != 0 && !accounts.check(user, &lines[0]) {
                std::thread::sleep(std::time::Duration::from_secs(2));
                bail!("the current password is wrong");
            }
            accounts.set_password(user, &lines[1])
        }
        "chfn" => {
            let user = arg(1)?;
            allowed(&accounts, user)?;
            accounts.set_real_name(user, &args[2..].join(" "))
        }
        cmd @ ("lock" | "unlock") => {
            if caller() != 0 {
                bail!("only root can lock or unlock accounts");
            }
            accounts.set_locked(arg(1)?, cmd == "lock")
        }
        other => bail!("unknown command {other}"),
    }
}
