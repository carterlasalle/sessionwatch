//! Append-only log tailing: reads only the bytes appended since the last
//! check (plus a small context window), so per-refresh IO stays tiny even on
//! year-old multi-MB logs like /var/log/wtmp. Detects rotation/truncation
//! (file shrank) and rebuilds from scratch.
#![allow(dead_code)] // used by the Linux collectors; kept compiled for tests

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Context bytes kept before the read offset so records that straddle the
/// incremental-read boundary (e.g. a login at the end of one chunk and its
/// logout at the start of the next) still get paired.
const CONTEXT_BYTES: u64 = 4096;

pub struct IncrementalFile {
    path: PathBuf,
    last_len: u64,
}

impl IncrementalFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        IncrementalFile {
            path: path.into(),
            last_len: 0,
        }
    }

    /// Return the new bytes since the previous call (or the whole file on
    /// first call / after rotation). Empty on error.
    pub fn tail(&mut self) -> Vec<u8> {
        let Ok(md) = std::fs::metadata(&self.path) else {
            self.last_len = 0;
            return Vec::new();
        };
        let len = md.len();
        if len < self.last_len {
            self.last_len = 0; // rotated or truncated
        }
        let start = self.last_len.saturating_sub(CONTEXT_BYTES);
        let mut out = Vec::with_capacity((len - start) as usize);
        if let Ok(mut f) = std::fs::File::open(&self.path) {
            if f.seek(SeekFrom::Start(start)).is_err() {
                return Vec::new();
            }
            if f.read_to_end(&mut out).is_err() {
                return Vec::new();
            }
            self.last_len = len;
        }
        out
    }

    /// Forget everything (e.g. file disappeared).
    pub fn reset(&mut self) {
        self.last_len = 0;
    }
}
