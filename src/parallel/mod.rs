//! Parallel Account API prepaid balance integration.

pub mod credentials;
pub mod fetch;
pub mod types;
pub mod vendor;

pub use fetch::fetch_snapshot;
