//! System statistics and privacy probes for the dashboard.

pub mod battery;
pub mod collector;
pub mod privacy;

pub use collector::{DiskInfo, ProcInfo, SysSnapshot, SysmonCollector};
pub use privacy::{PrivacyProbe, PrivacyStatus};
