use overload::overload;
use serde::Deserialize;
use serde::Serialize;
use std::ops;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn neg(&self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

overload!((a: f64) * (b: Side) -> f64 { if b == Side::Buy { a } else { -a } });

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub symbol: String,
    pub side: Side,
    pub trigger_price: f64,
    pub price: f64,
    pub quantity: f64,
    pub reduce_only: bool,
}

impl Order {
    pub fn is_marker(&self) -> bool {
        self.trigger_price == 0.0 && self.price == 0.0
    }

    pub fn is_limit(&self) -> bool {
        self.trigger_price == 0.0 && self.price != 0.0
    }

    pub fn is_trigger(&self) -> bool {
        self.trigger_price != 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// The order has been submitted but not yet processed/matched by the exchange.
    Submitted,

    /// The order has been partially filled (some quantity remains unmatched).
    PartiallyFilled,

    /// The order has been completely filled (all quantity has been executed).
    Filled,

    /// The order has been actively cancelled by the user, or cancelled by the system (e.g., reduce-only, timeout, risk control trigger).
    Canceled,

    /// The order has been rejected, typically due to parameter errors, insufficient balance, or violation of trading rules.
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Trigger,
    Marker,
    Limit,
    Liquidation,
    ADL,
}

impl Kind {
    pub fn is_normal(&self) -> bool {
        matches!(self, Kind::Trigger | Kind::Marker | Kind::Limit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderMessage {
    pub symbol: String,
    pub side: Side,
    pub trigger_price: f64,
    pub price: f64,
    pub quantity: f64,
    pub reduce_only: bool,
    pub id: String,
    pub kind: Kind,
    pub avg_price: f64,
    pub cumulative_quantity: f64,
    pub create_time: u64,
    pub update_time: u64,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub leverage: u32,
    pub side: Side,
    pub open_avg_price: f64,
    pub quantity: f64,
    pub margin: f64,
    pub liquidation_price: f64,
    pub profit: f64,
    pub open_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPosition {
    pub symbol: String,
    pub leverage: u32,
    pub side: Side,
    pub open_avg_price: f64,
    pub close_avg_price: f64,
    pub max_quantity: f64,
    pub close_quantity: f64,
    pub total_profit: f64,
    pub profit: f64,
    pub fee: f64,
    pub open_time: u64,
    pub close_time: u64,
    pub log: Vec<Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPositionSummary {
    pub symbol: String,
    pub leverage: u32,
    pub total_trades: usize,
    pub win_rate: f64,
    pub win_trades: usize,
    pub loss_trades: usize,
    pub total_profit: f64,
    pub profit_loss_ratio: f64,
    pub net_gross_profit: f64,
    pub net_gross_loss_abs: f64,
    pub gross_profit: f64,
    pub gross_loss_abs: f64,
    pub total_fee: f64,
    pub avg_profit: f64,
    pub best_trade: f64,
    pub worst_trade: f64,
}

impl HistoryPosition {
    pub fn is_liquidation(&self) -> bool {
        self.log
            .last()
            .map(|v| v.kind == Kind::Liquidation)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub kind: Kind,
    pub side: Side,
    pub price: f64,
    pub quantity: f64,
    pub profit: f64,
    pub fee: f64,
    pub time: u64,
}
