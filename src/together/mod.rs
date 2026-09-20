//! Together AI organization billing usage.
//!
//! The documented beta endpoint returns finalized daily line items for the
//! current billing month. It does not expose the prepaid credit balance shown
//! in the Together dashboard, so this provider reports spend only.

pub mod fetch;
pub mod types;
pub mod vendor;

pub use fetch::{FetchOutcome, fetch_snapshot};
