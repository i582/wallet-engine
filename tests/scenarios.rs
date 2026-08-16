#![allow(
    unused_results,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::iter_over_hash_type,
    clippy::panic,
    clippy::pedantic,
    reason = "the scenario harness uses panic-based assertions and intentionally concise fixture code"
)]

mod support;

#[path = "scenarios/lifecycle.rs"]
mod lifecycle;
#[path = "scenarios/pagination.rs"]
mod pagination;
#[path = "scenarios/refresh.rs"]
mod refresh;
#[path = "scenarios/send.rs"]
mod send;
#[path = "scenarios/wallet_lifecycle.rs"]
mod wallet_lifecycle;
