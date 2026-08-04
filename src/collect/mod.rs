//! Collector abstraction over "what is happening on the machine".
//!
//! On Linux we read `/proc` + utmpx directly. Elsewhere (and with `--demo`)
//! we synthesize a realistic stream so the TUI is runnable/screenshot-able
//! anywhere.

pub mod demo;
pub mod linux;
pub mod utmp;

use crate::model::Snapshot;

/// Produces a [`Snapshot`] on demand. Implementations keep whatever state they
/// need to compute deltas (e.g. per-pid CPU ticks).
pub trait Collector: Send {
    fn collect(&mut self) -> Snapshot;
    /// Human-readable source name, shown in the header (e.g. "live:/proc" or "demo").
    fn source(&self) -> &'static str;
}

/// Build the collector. On Linux the real one; `demo=true` (or a non-Linux host)
/// selects the synthetic generator.
pub fn new(demo: bool) -> Box<dyn Collector> {
    #[cfg(target_os = "linux")]
    {
        if !demo {
            return Box::new(linux::LinuxCollector::new());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = demo;
    }
    Box::new(demo::DemoCollector::new())
}
