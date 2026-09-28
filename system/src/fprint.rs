//! Fingerprint enrolment via fprintd's D-Bus API (net.reactivated.Fprint).

use anyhow::{anyhow, Context, Result};
use zbus::blocking::Connection;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FprintState {
    pub available: bool,
    pub device: String,
    pub enrolled: Vec<String>,
}

pub const FINGERS: [&str; 10] = [
    "left-thumb",
    "left-index-finger",
    "left-middle-finger",
    "left-ring-finger",
    "left-little-finger",
    "right-thumb",
    "right-index-finger",
    "right-middle-finger",
    "right-ring-finger",
    "right-little-finger",
];

fn device(conn: &Connection) -> Result<zbus::blocking::Proxy<'static>> {
    let manager = zbus::blocking::Proxy::new(
        conn,
        "net.reactivated.Fprint",
        "/net/reactivated/Fprint/Manager",
        "net.reactivated.Fprint.Manager",
    )?;
    let path: zbus::zvariant::OwnedObjectPath = manager.call("GetDefaultDevice", &())?;
    Ok(zbus::blocking::Proxy::new(
        conn,
        "net.reactivated.Fprint",
        path,
        "net.reactivated.Fprint.Device",
    )?)
}

pub fn query(user: &str) -> FprintState {
    let Ok(conn) = Connection::system() else {
        return FprintState::default();
    };
    let Ok(dev) = device(&conn) else {
        return FprintState::default();
    };
    let name: String = dev.get_property("name").unwrap_or_default();
    let enrolled: Vec<String> = dev
        .call("ListEnrolledFingers", &(user,))
        .unwrap_or_default();
    FprintState {
        available: true,
        device: name,
        enrolled,
    }
}

/// Delete all enrolled fingerprints for the user.
pub fn delete_all(user: &str) -> Result<()> {
    let conn = Connection::system()?;
    let dev = device(&conn)?;
    dev.call::<_, _, ()>("Claim", &(user,))?;
    let res = dev
        .call::<_, _, ()>("DeleteEnrolledFingers2", &())
        .or_else(|_| dev.call::<_, _, ()>("DeleteEnrolledFingers", &(user,)));
    let _ = dev.call::<_, _, ()>("Release", &());
    res.context("deleting fingerprints")
}

/// Enrol a finger interactively; `progress` receives fprintd status strings
/// ("enroll-stage-passed", "enroll-completed", …). Blocks until completion.
pub fn enroll(user: &str, finger: &str, progress: &dyn Fn(String)) -> Result<()> {
    if !FINGERS.contains(&finger) {
        return Err(anyhow!("unknown finger {finger}"));
    }
    let conn = Connection::system()?;
    let dev = device(&conn)?;
    dev.call::<_, _, ()>("Claim", &(user,))?;
    let result = (|| -> Result<()> {
        dev.call::<_, _, ()>("EnrollStart", &(finger,))?;
        let signals = dev.receive_signal("EnrollStatus")?;
        for msg in signals {
            let (status, done): (String, bool) = msg.body().deserialize()?;
            progress(status.clone());
            if done {
                dev.call::<_, _, ()>("EnrollStop", &())?;
                if status == "enroll-completed" {
                    return Ok(());
                }
                return Err(anyhow!("enrolment ended: {status}"));
            }
        }
        Err(anyhow!("fprintd signal stream ended"))
    })();
    let _ = dev.call::<_, _, ()>("Release", &());
    result
}
