use overload::overload;
use rust_decimal::Decimal;
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

overload!((a: Decimal) * (b: Side) -> Decimal { if b == Side::Buy { a } else { -a } });

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub symbol: String,
    pub side: Side,
    pub trigger_price: Decimal,
    pub price: Decimal,
    pub quantity: Decimal,
    pub reduce_only: bool,
}

impl Order {
    pub fn is_market(&self) -> bool {
        self.trigger_price == Decimal::ZERO && self.price == Decimal::ZERO
    }

    pub fn is_limit(&self) -> bool {
        self.trigger_price == Decimal::ZERO && self.price != Decimal::ZERO
    }

    pub fn is_trigger(&self) -> bool {
        self.trigger_price != Decimal::ZERO
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

impl Status {
    pub fn is_valid(&self) -> bool {
        !matches!(self, Status::Canceled | Status::Rejected)
    }

    pub fn ok(&self) -> anyhow::Result<()> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(anyhow::anyhow!("order is not valid: {:?}", self))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Trigger,
    Market,
    Limit,
    Liquidation,
    ADL,
}

impl Kind {
    pub fn is_normal(&self) -> bool {
        matches!(self, Kind::Trigger | Kind::Market | Kind::Limit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderMessage {
    pub symbol: String,
    pub side: Side,
    pub trigger_price: Decimal,
    pub price: Decimal,
    pub quantity: Decimal,
    pub reduce_only: bool,
    pub id: String,
    pub kind: Kind,
    pub avg_price: Decimal,
    pub cumulative_quantity: Decimal,
    pub create_time: u64,
    pub update_time: u64,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub leverage: u32,
    pub side: Side,
    pub open_avg_price: Decimal,
    pub quantity: Decimal,
    pub margin: Decimal,
    pub liquidation_price: Decimal,
    pub profit: Decimal,
    pub open_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPosition {
    pub symbol: String,
    pub leverage: u32,
    pub side: Side,
    pub open_avg_price: Decimal,
    pub close_avg_price: Decimal,
    pub max_quantity: Decimal,
    pub close_quantity: Decimal,
    pub total_profit: Decimal,
    pub profit: Decimal,
    pub fee: Decimal,
    pub open_time: u64,
    pub close_time: u64,
    pub log: Vec<Record>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPositionSummary {
    pub symbol: String,
    pub leverage: u32,
    pub total_trades: usize,
    pub win_rate: Decimal,
    pub win_trades: usize,
    pub loss_trades: usize,
    pub total_profit: Decimal,
    pub profit_loss_ratio: Decimal,
    pub net_gross_profit: Decimal,
    pub net_gross_loss_abs: Decimal,
    pub gross_profit: Decimal,
    pub gross_loss_abs: Decimal,
    pub total_fee: Decimal,
    pub avg_profit: Decimal,
    pub best_trade: Decimal,
    pub worst_trade: Decimal,
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
    pub price: Decimal,
    pub quantity: Decimal,
    pub profit: Decimal,
    pub fee: Decimal,
    pub time: u64,
}
