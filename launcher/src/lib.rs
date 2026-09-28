//! Application launcher: XDG desktop entry scanning, fuzzy search, detached launching.

pub mod desktop;
pub mod history;
pub mod runner;
pub mod search;

pub use desktop::{scan_applications, AppEntry};
pub use history::LaunchHistory;
pub use runner::{launch, LaunchOptions};
pub use search::AppSearch;
