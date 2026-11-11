//! Clocks. Native code uses `std::time`. On wasm32-unknown-unknown, `SystemTime::now` and
//! `Instant::now` in std panic (`time not implemented on this platform`); `web-time` reads the
//! page's clock instead.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
pub use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};
