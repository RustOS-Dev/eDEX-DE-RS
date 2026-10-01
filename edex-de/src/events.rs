//! Events delivered into the shell's main loop from threads, sockets and timers.

use notifications::server::ServerEvent;
use system::SysReply;
use terminal::TermEvent;

#[derive(Debug)]
pub enum AppEvent {
    Term(TermEvent),
    Notify(ServerEvent),
    Sys(SysReply),
    CompReadable,
    IpcReadable,
    Tick(Tick),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// Once a second: clock, config watcher, history flush.
    Clock,
    /// Once a second: sysmon refresh.
    Sysmon,
    /// Every three seconds: privacy probes and status polling.
    Privacy,
    /// Animation tick (border pulse, boot animation, cursor blink).
    Anim,
    /// Toast/OSD expiry.
    Toast,
}
