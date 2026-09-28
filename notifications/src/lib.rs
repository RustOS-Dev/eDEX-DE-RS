//! `org.freedesktop.Notifications` server and notification store for eDEX-DE.

pub mod server;
pub mod store;

pub use server::{NotificationServer, ServerEvent, CLOSE_DISMISSED, CLOSE_EXPIRED, CLOSE_REQUESTED};
pub use store::{Notification, NotificationStore, Urgency};
