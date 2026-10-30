//! # trading-maid
//!
//! A high-fidelity crypto futures backtesting and live trading framework with matching, margin,
//! leverage and liquidation simulation.
//!
//! Most users only need `use trading_maid::prelude::*;`, which re-exports every public module
//! together with `anyhow`, `async_trait`, `rust_decimal` and `rust_decimal_macros`.

pub use async_trait;
pub mod context;
pub mod data;
pub mod engine;
pub mod exchange;
pub mod indicator;
pub mod local_exchange;
pub mod order;
pub mod series;
pub mod strategy;
pub mod util;

pub use anyhow;
pub use rust_decimal;
pub use rust_decimal_macros;

/// Everything needed for a typical strategy: all public modules plus the external crates used in
/// their signatures.
pub mod prelude {
    pub use crate::context::*;
    pub use crate::data::*;
    pub use crate::engine::*;
    pub use crate::exchange::*;
    pub use crate::indicator::*;
    pub use crate::local_exchange::*;
    pub use crate::order::*;
    pub use crate::series::*;
    pub use crate::strategy::*;
    pub use crate::util::*;
    pub use anyhow;
    pub use async_trait::*;
    pub use rust_decimal::*;
    pub use rust_decimal_macros::*;
}
