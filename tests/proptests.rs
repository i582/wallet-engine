#![allow(
    dead_code,
    unused_imports,
    unused_results,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::iter_over_hash_type,
    clippy::panic,
    clippy::pedantic,
    reason = "the property harness uses panic-based assertions and intentionally concise fixture code"
)]

mod support;

#[path = "proptests/refresh.rs"]
mod refresh;
