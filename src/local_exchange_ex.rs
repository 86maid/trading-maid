use anyhow::Context;
use anyhow::bail;
use indexmap::IndexMap;
use inherits::inherits;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::*;
use crate::exchange::*;
use crate::order::*;
use crate::util::*;

/// A multi-symbol backtesting exchange that replays historical K-line data.
///
/// `LocalExchangeEx` steps through multiple [`DataSource`]s synchronised via [`Axis`].
/// Each call to [`Exchange::next`] returns the current K-line for the given symbol;
/// the first call in a round advances the global timeline, and the last call
/// (when all symbols have been queried) resets the round counter.
///
/// # Builder pattern
///
/// After [`LocalExchangeEx::new`], chain any of the following:
/// - [`cash`](LocalExchangeEx::cash) — starting balance (default: 10,000).
/// - [`leverage`](LocalExchangeEx::leverage) — default leverage for all symbols (default: 1).
/// - [`slippage`](LocalExchangeEx::slippage) — fraction applied to market-order fill prices (default: 0).
/// - [`range`](LocalExchangeEx::range) — restrict replayed data to `[start_time, end_time)`.
///
/// # Thread safety
///
/// All public methods take `&self` and lock an internal `Arc<Mutex<…>>`.
#[derive(Clone)]
pub struct LocalExchangeEx {
    inner: Arc<Mutex<LocalExchangeExInner>>,
}

struct LocalExchangeExInner {
    axis: Axis,
    /// Current kline per symbol, updated each round.
    klines: BTreeMap<String, KLine>,
    cash: Decimal,
    /// Global default leverage. Per-symbol overrides live in `leverages`.
    default_leverage: u32,
    leverages: BTreeMap<String, u32>,
    slippage: Decimal,
    history_order_list: IndexMap<String, OrderEx>,
    pending_order_list: IndexMap<String, OrderEx>,
    positions: BTreeMap<String, PositionEx>,
    history_position_list: Vec<HistoryPosition>,
    id: u64,
    /// A1 pacemaker: the first symbol ever passed to `next()`. It drives the
    /// axis — only calls from the pacemaker advance the timeline.
    pacemaker: Option<String>,
    /// Set by the pacemaker when the axis is exhausted. All symbols check this
    /// to return `None` uniformly.
    exhausted: bool,
}

#[inherits(Order)]
#[derive(Clone)]
struct OrderEx {
    id: String,
    kind: Kind,
    avg_price: Decimal,
    cumulative_quantity: Decimal,
    create_time: u64,
    update_time: u64,
    status: Status,
    freeze_margin: Decimal,
}

impl OrderEx {
    fn to_order_message(&self) -> OrderMessage {
        OrderMessage {
            symbol: self.symbol.clone(),
            side: self.side,
            trigger_price: self.trigger_price,
            price: self.price,
            quantity: self.quantity,
            reduce_only: self.reduce_only,
            id: self.id.clone(),
            kind: self.kind,
            avg_price: self.avg_price,
            cumulative_quantity: self.cumulative_quantity,
            create_time: self.create_time,
            update_time: self.update_time,
            status: self.status,
        }
    }
}

#[inherits(Position)]
#[derive(Debug)]
struct PositionEx {
    liquidation_order_id: String,
    log: Vec<Record>,
}

impl PositionEx {
    fn calc_max_quantity(&self) -> Decimal {
        let mut max_quantity = None;
        let mut sum = Decimal::ZERO;
        for v in self.log.iter() {
            sum += v.quantity * v.side;
            let exposure = sum.abs();
            if let Some(max_quantity) = &mut max_quantity {
                if exposure > *max_quantity {
                    *max_quantity = exposure;
                }
            } else {
                max_quantity = Some(exposure);
            }
        }
        max_quantity.unwrap_or(Decimal::ZERO)
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

impl LocalExchangeEx {
    /// Creates a new `LocalExchangeEx` from a list of [`DataSource`]s.
    ///
    /// All data sources must share the same [`Level`] and have overlapping
    /// time ranges. The shortest data source determines the total steps.
    ///
    /// # Panics
    ///
    /// Panics if `data_sources` is empty, levels differ, or time ranges
    /// do not overlap.
    pub fn new(data_sources: Vec<DataSource>) -> Self {
        let axis = Axis::new(data_sources).expect("LocalExchangeEx::new: Axis creation failed");

        let symbols: Vec<String> = axis
            .inner()
            .iter()
            .map(|ds| ds.metadata.symbol.clone())
            .collect();

        let mut klines = BTreeMap::new();
        let mut leverages = BTreeMap::new();
        for sym in &symbols {
            klines.insert(sym.clone(), KLine::default());
            leverages.insert(sym.clone(), 1);
        }

        Self {
            inner: Arc::new(Mutex::new(LocalExchangeExInner {
                axis,
                klines,
                cash: Decimal::from(10000),
                default_leverage: 1,
                leverages,
                slippage: Decimal::ZERO,
                positions: BTreeMap::new(),
                history_order_list: IndexMap::new(),
                pending_order_list: IndexMap::new(),
                history_position_list: Vec::new(),
                id: 0,
                pacemaker: None,
                exhausted: false,
            })),
        }
    }

    /// Sets the starting cash balance (shared across all symbols).
    pub fn cash(self, cash: impl TryInto<Decimal>) -> Self {
        self.inner.try_lock().unwrap().cash =
            cash.try_into().unwrap_or_else(|_| panic!("invalid cash"));
        self
    }

    /// Sets the default leverage for all symbols.
    ///
    /// Must be ≥ 1. Individual symbol leverage can be changed later via
    /// [`Exchange::set_leverage`].
    pub fn leverage(self, leverage: u32) -> Self {
        let mut inner = self.inner.try_lock().unwrap();
        inner.default_leverage = leverage;
        for lev in inner.leverages.values_mut() {
            *lev = leverage;
        }
        drop(inner);
        self
    }

    /// Sets the slippage fraction applied to market-order fill prices.
    pub fn slippage(self, slippage: impl TryInto<Decimal>) -> Self {
        self.inner.try_lock().unwrap().slippage = slippage
            .try_into()
            .unwrap_or_else(|_| panic!("invalid slippage"));
        self
    }

    /// Restricts the replayed data for all symbols to candles whose `time`
    /// falls in `[start_time, end_time]` (inclusive on both sides, matching
    /// [`DataSource::range`]).
    ///
    /// Rebuilds the [`Axis`] from the filtered data sources.
    pub fn range(self, start_time: u64, end_time: u64) -> Self {
        let mut inner = self.inner.try_lock().unwrap();

        let filtered: Vec<DataSource> = inner
            .axis
            .inner()
            .iter()
            .map(|ds| ds.range(start_time, end_time))
            .collect();

        inner.axis = Axis::new(filtered).expect("range: Axis rebuild failed");

        drop(inner);
        self
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

impl LocalExchangeExInner {
    /// Look up metadata for a symbol from the Axis.
    fn metadata(&self, symbol: &str) -> Option<&Metadata> {
        self.axis
            .inner()
            .iter()
            .find(|ds| ds.metadata.symbol == symbol)
            .map(|ds| &ds.metadata)
    }

    /// Get the current kline for a symbol.
    fn kline(&self, symbol: &str) -> &KLine {
        // The klines map is populated during next(); if missing, return a
        // zeroed default (should not happen in normal operation).
        static DEFAULT: KLine = KLine {
            time: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: Decimal::ZERO,
        };
        self.klines.get(symbol).unwrap_or(&DEFAULT)
    }

    fn leverage(&self, symbol: &str) -> u32 {
        self.leverages.get(symbol).copied().unwrap_or(self.default_leverage)
    }

    fn calc_market_price_slippage(&self, symbol: &str, side: Side, market_price: Decimal) -> Decimal {
        let kline = self.kline(symbol);
        let price: Decimal = match side {
            Side::Buy => market_price * (Decimal::ONE + self.slippage),
            Side::Sell => market_price * (Decimal::ONE - self.slippage),
        };

        let low = kline.low.min(kline.high);
        let high = kline.low.max(kline.high);

        price.clamp(low, high)
    }

    fn freeze_margin(&mut self, order: &mut OrderEx, leverage: u32) -> anyhow::Result<()> {
        let freeze_margin = calc_initial_margin(order.price, order.quantity, leverage);
        self.need_cash(freeze_margin)?;
        order.freeze_margin = freeze_margin;
        Ok(())
    }

    fn adjust_freeze_margin_by_avg_price(
        &mut self,
        order: &mut OrderEx,
        leverage: u32,
    ) -> anyhow::Result<()> {
        if order.reduce_only || order.freeze_margin == Decimal::ZERO {
            return Ok(());
        }

        let filled_margin = calc_initial_margin(order.avg_price, order.quantity, leverage);

        if order.freeze_margin > filled_margin {
            self.cash += order.freeze_margin - filled_margin;
        } else if order.freeze_margin < filled_margin {
            self.need_cash(filled_margin - order.freeze_margin)?;
        }

        order.freeze_margin = filled_margin;
        Ok(())
    }

    fn need_cash(&mut self, need: Decimal) -> anyhow::Result<()> {
        if !need.is_zero() && self.cash < need {
            bail!("cash shortage, need {}, balance {}", need, self.cash);
        }
        self.cash -= need;
        Ok(())
    }

    fn place_order(&mut self, order: Order, kind: Kind) -> anyhow::Result<String> {
        let leverage = self.leverage(&order.symbol);
        let kline_time = self.kline(&order.symbol).time;
        let id = t2s(kline_time) + " [" + self.id.to_string().as_str() + "]";
        self.id += 1;

        let mut order = OrderEx {
            parent: order,
            id: id.clone(),
            kind,
            avg_price: Decimal::ZERO,
            cumulative_quantity: Decimal::ZERO,
            create_time: kline_time,
            update_time: kline_time,
            status: Status::Submitted,
            freeze_margin: Decimal::ZERO,
        };

        if !order.reduce_only && order.is_limit() {
            self.freeze_margin(&mut order, leverage)
                .context(format!("place_order: {}", order.symbol))?;
        }

        self.pending_order_list.insert(id.clone(), order);
        Ok(id)
    }

    /// Cache klines at the current axis index, then advance.
    fn advance(&mut self) {
        // Read all klines at the current index BEFORE advancing.
        if let Some(all_klines) = self.axis.all() {
            for (sym, kline) in all_klines {
                self.klines.insert(sym.to_string(), kline);
            }
        }
        // Advance for the next round.
        self.axis.next();
        self.update();
    }

    fn update(&mut self) {
        // ---- process pending orders ----
        let mut normal_queue = VecDeque::new();
        let mut liquidation_queue = VecDeque::new();

        for (id, order) in self.pending_order_list.iter() {
            if order.kind.is_normal() {
                normal_queue.push_back(id.clone());
            } else {
                liquidation_queue.push_back(id.clone());
            }
        }

        while let Some(id) = normal_queue.pop_front() {
            self.update_order(&id, &mut normal_queue);
        }

        while let Some(id) = liquidation_queue.pop_front() {
            self.update_order(&id, &mut normal_queue);
        }

        // ---- update PnL and liquidation prices for all positions ----
        // Pre-compute updates to avoid simultaneous mutable/immutable borrow.
        let position_pnl: Vec<(String, Decimal, Decimal)> = self
            .positions
            .iter()
            .map(|(sym, pos)| {
                let kline = self.kline(sym);
                let metadata = self.metadata(sym).expect("metadata not found");
                let profit = if pos.side == Side::Buy {
                    (kline.close - pos.open_avg_price) * pos.quantity
                } else {
                    (pos.open_avg_price - kline.close) * pos.quantity
                };
                let liq_price = calc_liquidation_price(
                    pos.leverage,
                    metadata.maintenance,
                    pos.side,
                    pos.open_avg_price,
                    pos.quantity,
                    pos.margin,
                );
                (sym.clone(), profit, liq_price)
            })
            .collect();

        for (sym, profit, liq_price) in position_pnl {
            if let Some(pos) = self.positions.get_mut(&sym) {
                pos.profit = profit;
                pos.liquidation_price = liq_price;
            }
        }
    }

    fn shift_remove_order(&mut self, order_id: &str) -> Option<OrderEx> {
        self.pending_order_list.shift_remove(order_id)
    }

    fn handle_trigger_order(&mut self, order_id: &str, order_queue: &mut VecDeque<String>) {
        let mut order_ref = match self.shift_remove_order(order_id) {
            Some(v) => v,
            None => return,
        };

        let result = self.place_order(
            Order {
                symbol: order_ref.symbol.clone(),
                side: order_ref.side,
                trigger_price: Decimal::ZERO,
                price: if order_ref.price == Decimal::ZERO {
                    order_ref.trigger_price
                } else {
                    order_ref.price
                },
                quantity: order_ref.quantity,
                reduce_only: order_ref.reduce_only,
            },
            if order_ref.price == Decimal::ZERO {
                Kind::Market
            } else {
                Kind::Limit
            },
        );

        let kline_time = self.kline(&order_ref.symbol).time;
        order_ref.update_time = kline_time;
        order_ref.status = if let Ok(v) = result {
            order_queue.push_back(v);
            Status::Filled
        } else {
            Status::Rejected
        };

        self.history_order_list
            .insert(order_ref.id.clone(), order_ref);
    }

    fn handle_limit_or_liquidation_order(&mut self, order_id: &str) {
        let order_ref = self.try_fill_limit_order(order_id);
        let Some(order_ref) = order_ref else {
            return;
        };

        let sym = order_ref.symbol.clone();
        let fee_rate = if order_ref.kind == Kind::Liquidation {
            self.metadata(&sym).map(|m| m.taker_fee).unwrap_or_default()
        } else {
            self.metadata(&sym).map(|m| m.maker_fee).unwrap_or_default()
        };

        self.execute_order(order_id, order_ref, fee_rate);
    }

    fn try_fill_limit_order(&mut self, order_id: &str) -> Option<OrderEx> {
        let order = self.pending_order_list.get(order_id)?;
        // Copy the kline to release the immutable borrow before mutable ops below.
        let kline = *self.kline(&order.symbol);

        if order.kind == Kind::Liquidation {
            if !(order.price >= kline.low && order.price <= kline.high) {
                return None;
            }
            let mut order_ref = self.shift_remove_order(order_id)?;
            order_ref.avg_price = order_ref.price;
            Some(order_ref)
        } else if (order.side == Side::Buy && order.price >= kline.open)
            || (order.side == Side::Sell && order.price <= kline.open)
        {
            let mut order_ref = self.shift_remove_order(order_id)?;
            order_ref.avg_price = if order_ref.side == Side::Buy {
                kline.high
            } else {
                kline.low
            };
            Some(order_ref)
        } else if (order.side == Side::Buy && kline.low <= order.price)
            || (order.side == Side::Sell && kline.high >= order.price)
        {
            let mut order_ref = self.shift_remove_order(order_id)?;
            order_ref.avg_price = order_ref.price;
            Some(order_ref)
        } else {
            None
        }
    }

    fn handle_market_order(&mut self, order_id: &str) {
        let mut order_ref = match self.shift_remove_order(order_id) {
            Some(v) => v,
            None => return,
        };

        let sym = order_ref.symbol.clone();
        let kline = self.kline(&sym);

        if order_ref.price == Decimal::ZERO {
            order_ref.price = self.calc_market_price_slippage(&sym, order_ref.side, kline.open);
        } else {
            order_ref.price = self.calc_market_price_slippage(&sym, order_ref.side, order_ref.price);
        }

        order_ref.avg_price = order_ref.price;
        let fee_rate = self.metadata(&sym).map(|m| m.taker_fee).unwrap_or_default();
        self.execute_order(order_id, order_ref, fee_rate);
    }

    fn update_order(&mut self, order_id: &str, order_queue: &mut VecDeque<String>) {
        let Some(order) = self.pending_order_list.get(order_id) else {
            return;
        };

        if order.status != Status::Submitted {
            return;
        }

        let sym = order.symbol.clone();
        let kline = self.kline(&sym);

        if order.kind == Kind::Trigger {
            if !(order.trigger_price >= kline.low && order.trigger_price <= kline.high) {
                return;
            }
            self.handle_trigger_order(order_id, order_queue);
        } else if order.kind == Kind::Limit || order.kind == Kind::Liquidation {
            self.handle_limit_or_liquidation_order(order_id);
        } else {
            self.handle_market_order(order_id);
        }
    }

    // ---- position management (mostly unchanged, but uses per-symbol lookups) ----

    fn handle_reduce_only_checks(&mut self, order_ref: &mut OrderEx) -> bool {
        if let Some(v) = self.positions.get(&order_ref.symbol) {
            if v.side == order_ref.side {
                order_ref.status = Status::Canceled;
                self.history_order_list
                    .insert(order_ref.id.clone(), order_ref.clone());
                return true;
            } else {
                order_ref.quantity = order_ref.quantity.min(v.quantity);
            }
        } else {
            order_ref.status = Status::Canceled;
            order_ref.update_time = self.kline(&order_ref.symbol).time;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref.clone());
            return true;
        }
        false
    }

    fn handle_pre_execution_checks(&mut self, order_ref: &mut OrderEx, fee_rate: Decimal) -> bool {
        let sym = order_ref.symbol.clone();
        let leverage = self.leverage(&sym);
        let metadata = self.metadata(&sym).cloned().expect("metadata not found");

        if order_ref.reduce_only {
            if self.handle_reduce_only_checks(order_ref) {
                return false;
            }
        } else if order_ref.freeze_margin == Decimal::ZERO
            && self.freeze_margin(order_ref, leverage).is_err()
        {
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref.clone());
            return false;
        }

        if order_ref.avg_price != order_ref.price
            && self
                .adjust_freeze_margin_by_avg_price(order_ref, leverage)
                .is_err()
        {
            self.cash += order_ref.freeze_margin;
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref.clone());
            return false;
        }

        if order_ref.kind.is_normal()
            && metadata.min_notional != Decimal::ZERO
            && (order_ref.avg_price * order_ref.quantity) < metadata.min_notional
        {
            self.cash += order_ref.freeze_margin;
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref.clone());
            return false;
        }

        let fee_cost = order_ref.avg_price * order_ref.quantity * fee_rate;

        if self.need_cash(fee_cost).is_err() {
            if order_ref.kind.is_normal() {
                self.cash += order_ref.freeze_margin;
                order_ref.status = Status::Rejected;
                self.history_order_list
                    .insert(order_ref.id.clone(), order_ref.clone());
                return false;
            } else {
                self.cash -= fee_cost;
            }
        }

        true
    }

    fn calc_close_metrics(
        &self,
        position: &PositionEx,
        order_ref: &OrderEx,
        close_quantity: Decimal,
    ) -> (Decimal, Decimal) {
        let close_margin = position.margin * (close_quantity / position.quantity);
        let close_profit = if order_ref.kind == Kind::Liquidation {
            -close_margin
        } else if position.side == Side::Buy {
            (order_ref.avg_price - position.open_avg_price) * close_quantity
        } else {
            (position.open_avg_price - order_ref.avg_price) * close_quantity
        };
        (close_margin, close_profit)
    }

    fn push_history_position(
        &mut self,
        position: &PositionEx,
        order_ref: &OrderEx,
        close_quantity: Decimal,
        max_quantity: Decimal,
        profit: Decimal,
        fee: Decimal,
    ) {
        let kline_time = self.kline(&order_ref.symbol).time;
        self.history_position_list.push(HistoryPosition {
            symbol: position.symbol.clone(),
            leverage: position.leverage,
            side: position.side,
            open_avg_price: position.open_avg_price,
            close_avg_price: order_ref.avg_price,
            max_quantity,
            close_quantity,
            total_profit: profit - fee,
            profit,
            fee,
            open_time: position.open_time,
            close_time: kline_time,
            log: position.log.clone(),
        });
    }

    fn handle_close_position(
        &mut self,
        position: PositionEx,
        order_ref: &OrderEx,
        max_quantity: Decimal,
        profit_sum: Decimal,
        fee_sum: Decimal,
    ) {
        self.cash += order_ref.freeze_margin;

        // Extract values before mutable borrows / partial moves.
        let kline_time = self.kline(&order_ref.symbol).time;
        let sym = position.symbol.clone();

        if let Some(last) = self.history_position_list.iter_mut().last()
            && last.close_quantity != max_quantity
        {
            last.leverage = position.leverage;
            last.side = position.side;
            last.open_avg_price = position.open_avg_price;
            last.close_avg_price = order_ref.avg_price;
            last.max_quantity = max_quantity;
            last.close_quantity = max_quantity;
            last.total_profit = profit_sum - fee_sum;
            last.profit = profit_sum;
            last.fee = fee_sum;
            last.close_time = kline_time;
            last.log = position.log;
        } else {
            self.push_history_position(
                &position,
                order_ref,
                max_quantity,
                max_quantity,
                profit_sum,
                fee_sum,
            );
        }

        self.pending_order_list
            .shift_remove(&position.liquidation_order_id);
        self.positions.remove(&sym);
    }

    fn handle_partial_close_position(
        &mut self,
        mut position: PositionEx,
        order_ref: &OrderEx,
        max_quantity: Decimal,
        profit_sum: Decimal,
        fee_sum: Decimal,
    ) {
        self.cash += order_ref.freeze_margin;

        // Extract values before mutable borrows.
        let kline_time = self.kline(&order_ref.symbol).time;
        let sym = position.symbol.clone();

        let partial_close_qty = position
            .log
            .iter()
            .filter(|i| i.side != position.side)
            .map(|i| i.quantity)
            .sum::<Decimal>()
            .max(Decimal::ZERO);

        if let Some(last) = self.history_position_list.iter_mut().last()
            && last.close_quantity != last.max_quantity
        {
            last.leverage = position.leverage;
            last.side = position.side;
            last.open_avg_price = position.open_avg_price;
            last.max_quantity = max_quantity;
            last.close_avg_price = order_ref.avg_price;
            last.close_quantity = partial_close_qty;
            last.total_profit = profit_sum - fee_sum;
            last.profit = profit_sum;
            last.fee = fee_sum;
            last.close_time = kline_time;
            last.log = position.log.clone();
        } else {
            self.push_history_position(
                &position,
                order_ref,
                partial_close_qty,
                max_quantity,
                profit_sum,
                fee_sum,
            );
        }

        let metadata = self.metadata(&sym).cloned().expect("metadata not found");

        position.liquidation_price = calc_liquidation_price(
            position.leverage,
            metadata.maintenance,
            position.side,
            position.open_avg_price,
            position.quantity,
            position.margin,
        );

        self.pending_order_list
            .get_mut(&position.liquidation_order_id)
            .unwrap()
            .price = position.liquidation_price;

        self.positions.insert(sym, position);
    }

    fn handle_reverse_position(
        &mut self,
        id: &str,
        position: PositionEx,
        order_ref: &OrderEx,
        fee_rate: Decimal,
        remain_quantity: Decimal,
        max_quantity: Decimal,
        profit: Decimal,
        fee: Decimal,
    ) {
        let reverse_quantity = remain_quantity.abs();
        let sym = order_ref.symbol.clone();
        let leverage = self.leverage(&sym);
        let reverse_margin =
            calc_initial_margin(order_ref.avg_price, reverse_quantity, leverage);
        let kline_time = self.kline(&sym).time;
        // Read metadata now, before the mutable borrow on pending_order_list.
        let metadata = self.metadata(&sym).cloned().expect("metadata not found");
        let liquidation_order_id = position.liquidation_order_id.clone();

        self.cash += order_ref.freeze_margin - reverse_margin;

        self.history_position_list.push(HistoryPosition {
            symbol: position.symbol.clone(),
            leverage: position.leverage,
            side: position.side,
            open_avg_price: position.open_avg_price,
            close_avg_price: order_ref.avg_price,
            max_quantity,
            close_quantity: max_quantity,
            total_profit: profit - fee,
            profit,
            fee,
            open_time: position.open_time,
            close_time: kline_time,
            log: position.log,
        });

        let liquidation_order = self
            .pending_order_list
            .get_mut(&liquidation_order_id)
            .unwrap();

        liquidation_order.side = order_ref.side.neg();

        liquidation_order.price = calc_liquidation_price(
            leverage,
            metadata.maintenance,
            order_ref.side,
            order_ref.avg_price,
            reverse_quantity,
            reverse_margin,
        );

        self.positions.insert(
            sym.clone(),
            PositionEx {
                liquidation_order_id: liquidation_order.id.clone(),
                log: vec![Record {
                    id: id.to_string(),
                    kind: order_ref.kind,
                    side: order_ref.side,
                    price: order_ref.avg_price,
                    quantity: reverse_quantity,
                    profit: Decimal::ZERO,
                    fee: order_ref.avg_price * reverse_quantity * fee_rate,
                    time: kline_time,
                }],
                parent: Position {
                    symbol: sym,
                    leverage,
                    side: order_ref.side,
                    open_avg_price: order_ref.avg_price,
                    quantity: reverse_quantity,
                    margin: reverse_margin,
                    liquidation_price: liquidation_order.price,
                    profit: Decimal::ZERO,
                    open_time: kline_time,
                },
            },
        );
    }

    fn handle_open_position(&mut self, id: &str, order_ref: &OrderEx, fee_rate: Decimal) {
        let sym = order_ref.symbol.clone();
        let leverage = self.leverage(&sym);
        let metadata = self.metadata(&sym).cloned().expect("metadata not found");
        let kline_time = self.kline(&sym).time;

        let liquidation_price = calc_liquidation_price(
            leverage,
            metadata.maintenance,
            order_ref.side,
            order_ref.avg_price,
            order_ref.quantity,
            order_ref.freeze_margin,
        );

        let liquidation_order_id = self
            .place_order(
                Order {
                    symbol: sym.clone(),
                    side: order_ref.side.neg(),
                    trigger_price: Decimal::ZERO,
                    price: liquidation_price,
                    quantity: Decimal::MAX,
                    reduce_only: true,
                },
                Kind::Liquidation,
            )
            .unwrap();

        self.positions.insert(
            sym,
            PositionEx {
                liquidation_order_id,
                log: vec![Record {
                    id: id.to_string(),
                    kind: order_ref.kind,
                    side: order_ref.side,
                    price: order_ref.avg_price,
                    quantity: order_ref.quantity,
                    profit: Decimal::ZERO,
                    fee: order_ref.avg_price * order_ref.quantity * fee_rate,
                    time: kline_time,
                }],
                parent: Position {
                    symbol: order_ref.symbol.clone(),
                    leverage,
                    side: order_ref.side,
                    open_avg_price: order_ref.avg_price,
                    quantity: order_ref.quantity,
                    margin: order_ref.freeze_margin,
                    liquidation_price,
                    profit: Decimal::ZERO,
                    open_time: kline_time,
                },
            },
        );
    }

    fn execute_order(&mut self, id: &str, mut order_ref: OrderEx, fee_rate: Decimal) {
        let sym = order_ref.symbol.clone();
        let kline_time = self.kline(&sym).time;
        order_ref.update_time = kline_time;

        if !self.handle_pre_execution_checks(&mut order_ref, fee_rate) {
            return;
        }

        match self.positions.get(&sym) {
            Some(v) if v.side == order_ref.side => {
                let position = self.positions.remove(&sym).unwrap();
                self.handle_add_position(id, &order_ref, fee_rate, position);
            }
            Some(_) => {
                let mut position = self.positions.remove(&sym).unwrap();
                let close_quantity = order_ref.quantity.min(position.quantity);
                let remain_quantity = order_ref.quantity - position.quantity;
                let (close_margin, close_profit) =
                    self.calc_close_metrics(&position, &order_ref, close_quantity);

                position.quantity -= close_quantity;
                position.margin -= close_margin;
                self.cash += close_margin + close_profit;

                let close_fee = order_ref.avg_price * close_quantity * fee_rate;

                position.log.push(Record {
                    id: id.to_string(),
                    kind: order_ref.kind,
                    side: order_ref.side,
                    price: order_ref.avg_price,
                    quantity: order_ref.quantity,
                    profit: close_profit,
                    fee: close_fee,
                    time: kline_time,
                });

                let profit_sum = position.log.iter().map(|v| v.profit).sum();
                let fee_sum = position.log.iter().map(|v| v.fee).sum();
                let max_quantity = position.calc_max_quantity();

                if remain_quantity > Decimal::ZERO {
                    self.handle_reverse_position(
                        id,
                        position,
                        &order_ref,
                        fee_rate,
                        remain_quantity,
                        max_quantity,
                        profit_sum,
                        fee_sum,
                    );
                } else if remain_quantity == Decimal::ZERO {
                    self.handle_close_position(
                        position,
                        &order_ref,
                        max_quantity,
                        profit_sum,
                        fee_sum,
                    );
                } else {
                    self.handle_partial_close_position(
                        position,
                        &order_ref,
                        max_quantity,
                        profit_sum,
                        fee_sum,
                    );
                }
            }
            None => {
                self.handle_open_position(id, &order_ref, fee_rate);
            }
        }

        order_ref.status = Status::Filled;
        order_ref.cumulative_quantity = order_ref.quantity;
        self.history_order_list
            .insert(order_ref.id.clone(), order_ref);
    }

    fn handle_add_position(
        &mut self,
        id: &str,
        order_ref: &OrderEx,
        fee_rate: Decimal,
        mut position: PositionEx,
    ) {
        let sym = order_ref.symbol.clone();
        let metadata = self.metadata(&sym).cloned().expect("metadata not found");
        let kline_time = self.kline(&sym).time;

        let old_quantity = position.quantity;
        let new_quantity = old_quantity + order_ref.quantity;
        let new_avg_price = (old_quantity * position.open_avg_price
            + order_ref.quantity * order_ref.avg_price)
            / new_quantity;

        position.quantity = new_quantity;
        position.open_avg_price = new_avg_price;
        position.margin += order_ref.freeze_margin;

        position.liquidation_price = calc_liquidation_price(
            position.leverage,
            metadata.maintenance,
            position.side,
            position.open_avg_price,
            position.quantity,
            position.margin,
        );

        position.log.push(Record {
            id: id.to_string(),
            kind: order_ref.kind,
            side: order_ref.side,
            price: order_ref.avg_price,
            quantity: order_ref.quantity,
            profit: Decimal::ZERO,
            fee: order_ref.avg_price * order_ref.quantity * fee_rate,
            time: kline_time,
        });

        self.pending_order_list
            .get_mut(&position.liquidation_order_id)
            .unwrap()
            .price = position.liquidation_price;

        self.positions.insert(sym, position);
    }
}

// ---------------------------------------------------------------------------
// Exchange trait implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Exchange for LocalExchangeEx {
    async fn next(&self, symbol: &str, _level: Level) -> anyhow::Result<Option<KLine>> {
        let mut inner = self.inner.lock().await;

        // Verify this symbol is known
        if !inner.klines.contains_key(symbol) {
            bail!("next: unknown symbol: {}", symbol);
        }

        // Once the shared timeline is exhausted, all symbols return None.
        if inner.exhausted {
            return Ok(None);
        }

        // A1 pacemaker: the first symbol ever to call next() becomes the
        // pacemaker. Only pacemaker calls advance the timeline.
        if inner.pacemaker.is_none() {
            inner.pacemaker = Some(symbol.to_string());
        }

        let is_pacemaker = inner.pacemaker.as_deref() == Some(symbol);

        if is_pacemaker {
            if inner.axis.is_done() {
                inner.exhausted = true;
                return Ok(None);
            }
            inner.advance(); // axis.all() → klines cache → axis.next() → update()
        }

        Ok(inner.klines.get(symbol).cloned())
    }

    async fn place_order(&self, order: Order) -> anyhow::Result<String> {
        let metadata = self
            .get_metadata(&order.symbol)
            .await
            .context(format!("place_order: {}", order.symbol))?;

        if metadata.min_size <= Decimal::ZERO {
            bail!(
                "place_order: invalid metadata.min_size (must be > 0): {}",
                metadata.symbol
            );
        }

        if metadata.tick_size <= Decimal::ZERO {
            bail!(
                "place_order: invalid metadata.tick_size (must be > 0): {}",
                metadata.symbol
            );
        }

        if order.is_limit() && order.price <= Decimal::ZERO {
            bail!(
                "place_order: limit price must be greater than 0: {}",
                metadata.symbol
            );
        }

        if order.is_limit() && !is_tick_aligned(order.price, metadata.tick_size) {
            bail!(
                "place_order: limit price must align with metadata.tick_size {}: {}",
                metadata.tick_size,
                metadata.symbol
            );
        }

        if order.is_trigger() {
            if order.trigger_price <= Decimal::ZERO {
                bail!(
                    "place_order: trigger price must be greater than 0: {}",
                    metadata.symbol
                );
            }

            if order.price < Decimal::ZERO {
                bail!(
                    "place_order: trigger order price must be >= 0 (0 means trigger-market): {}",
                    metadata.symbol
                );
            }

            if !is_tick_aligned(order.trigger_price, metadata.tick_size) {
                bail!(
                    "place_order: trigger price must align with metadata.tick_size {}: {}",
                    metadata.tick_size,
                    metadata.symbol
                );
            }

            if order.price > Decimal::ZERO && !is_tick_aligned(order.price, metadata.tick_size) {
                bail!(
                    "place_order: trigger order price must align with metadata.tick_size {}: {}",
                    metadata.tick_size,
                    metadata.symbol
                );
            }
        }

        if order.quantity <= Decimal::ZERO {
            bail!(
                "place_order: quantity must be greater than 0: {}",
                metadata.symbol
            );
        }

        if !(order.reduce_only && order.quantity == Decimal::MAX) {
            if !is_tick_aligned(order.quantity, metadata.min_size) {
                bail!(
                    "place_order: quantity must be a multiple of metadata.min_size {}: {}",
                    metadata.min_size,
                    metadata.symbol
                );
            }

            if metadata.min_notional != Decimal::ZERO
                && order.is_limit()
                && (order.price * order.quantity) < metadata.min_notional
            {
                bail!(
                    "place_order: notional must be greater than metadata.min_notional {}: {}",
                    metadata.min_notional,
                    metadata.symbol
                );
            }
        }

        let kind = if order.is_trigger() {
            Kind::Trigger
        } else if order.is_limit() {
            Kind::Limit
        } else {
            Kind::Market
        };

        self.inner.lock().await.place_order(order, kind)
    }

    async fn cancel_order(&self, symbol: &str, id: &str) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("cancel_order: {}", symbol))?;

        let mut inner = self.inner.lock().await;

        let Some(order) = inner.pending_order_list.get(id) else {
            return Ok(());
        };

        if order.symbol != symbol {
            bail!("cancel_order: symbol mismatch: {}: {}", symbol, id);
        }

        if !order.kind.is_normal() {
            bail!(
                "cancel_order: can not cancel non-normal order: {}: {}",
                symbol,
                id
            );
        }

        if order.status != Status::Submitted {
            bail!("cancel_order: order is not submitted: {}: {}", symbol, id);
        }

        let Some(mut order) = inner.pending_order_list.shift_remove(id) else {
            bail!("cancel_order: no pending order: {}: {}", symbol, id);
        };

        order.status = Status::Canceled;
        order.update_time = inner.kline(symbol).time;

        inner.cash += order.freeze_margin;
        inner.history_order_list.insert(order.id.clone(), order);

        Ok(())
    }

    async fn cancel_all_order(&self, symbol: &str) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("cancel_all_order: {}", symbol))?;

        let mut inner = self.inner.lock().await;

        let id_list = inner
            .pending_order_list
            .iter()
            .filter(|(_, order)| {
                order.symbol == symbol
                    && order.status == Status::Submitted
                    && order.kind.is_normal()
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<String>>();

        for v in id_list {
            let Some(mut order) = inner.pending_order_list.shift_remove(&v) else {
                continue;
            };

            order.status = Status::Canceled;
            order.update_time = inner.kline(symbol).time;

            inner.cash += order.freeze_margin;
            inner.history_order_list.insert(order.id.clone(), order);
        }

        Ok(())
    }

    async fn get_order(&self, id: &str) -> anyhow::Result<Option<OrderMessage>> {
        let inner = self.inner.lock().await;

        Ok(inner
            .history_order_list
            .get(id)
            .or(inner.pending_order_list.get(id))
            .filter(|v| {
                if cfg!(debug_assertions) || cfg!(test) {
                    true
                } else {
                    v.kind.is_normal()
                }
            })
            .map(|v| v.to_order_message()))
    }

    async fn get_history_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_history_order_list: {}", symbol))?;

        Ok(self
            .inner
            .lock()
            .await
            .history_order_list
            .values()
            .filter(|v| v.symbol == symbol)
            .map(|v| v.to_order_message())
            .collect())
    }

    async fn get_pending_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_pending_order_list: {}", symbol))?;

        Ok(self
            .inner
            .lock()
            .await
            .pending_order_list
            .values()
            .filter(|v| {
                v.status == Status::Submitted
                    && if cfg!(debug_assertions) || cfg!(test) {
                        true
                    } else {
                        v.kind.is_normal()
                    }
            })
            .map(|v| v.to_order_message())
            .collect())
    }

    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_position: {}", symbol))?;

        Ok(self
            .inner
            .lock()
            .await
            .positions
            .get(symbol)
            .map(|v| v.parent.clone()))
    }

    async fn close_all_position(&self, symbol: &str) -> anyhow::Result<()> {
        let position = self
            .get_position(symbol)
            .await
            .context(format!("close_all_position: {}", symbol))?;

        if let Some(v) = position {
            self.place_order(Order {
                symbol: symbol.to_string(),
                side: v.side.neg(),
                trigger_price: Decimal::ZERO,
                price: Decimal::ZERO,
                quantity: Decimal::MAX,
                reduce_only: true,
            })
            .await?;
        }

        Ok(())
    }

    async fn get_history_position_list(
        &self,
        symbol: &str,
    ) -> anyhow::Result<Vec<HistoryPosition>> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_history_position: {}", symbol))?;

        Ok(self
            .inner
            .lock()
            .await
            .history_position_list
            .iter()
            .filter(|v| v.symbol == symbol)
            .cloned()
            .collect())
    }

    async fn append_position_margin(&self, symbol: &str, margin: Decimal) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("append_position_margin: {}", symbol))?;

        let mut inner = self.inner.lock().await;
        let cash = inner.cash;
        let metadata = inner.metadata(symbol).cloned().expect("metadata not found");

        let (liquidation_order_id, liquidation_price, cash_delta) =
            match inner.positions.get_mut(symbol) {
                Some(position) => {
                    let new_margin = position.margin + margin;
                    let init_margin = position.open_avg_price * position.quantity
                        / Decimal::from(position.leverage);

                    if new_margin < init_margin {
                        bail!(
                            "append_position_margin: {}: the initial margin of the position needs to be at least: {}",
                            symbol,
                            init_margin
                        );
                    }

                    if margin > Decimal::ZERO && cash < margin {
                        bail!(
                            "append_position_margin: {}: cash shortage, adjusting margin requires additional cash: {}",
                            symbol,
                            margin,
                        );
                    }

                    if margin < Decimal::ZERO && margin.abs() > position.margin {
                        bail!(
                            "append_position_margin: {}: cannot reduce margin more than current margin {}",
                            symbol,
                            position.margin
                        );
                    }

                    position.margin = new_margin;

                    position.liquidation_price = calc_liquidation_price(
                        position.leverage,
                        metadata.maintenance,
                        position.side,
                        position.open_avg_price,
                        position.quantity,
                        position.margin,
                    );

                    (
                        position.liquidation_order_id.clone(),
                        position.liquidation_price,
                        margin,
                    )
                }
                None => bail!("append_position_margin: no position: {}", symbol),
            };

        inner.cash -= cash_delta;

        if let Some(liquidation_order) =
            inner.pending_order_list.get_mut(&liquidation_order_id)
        {
            liquidation_order.price = liquidation_price;
        }

        Ok(())
    }

    async fn get_equity(&self) -> anyhow::Result<Decimal> {
        let inner = self.inner.lock().await;

        let pending_freeze_margin = inner
            .pending_order_list
            .values()
            .filter(|order| order.status == Status::Submitted && order.kind.is_normal())
            .map(|order| order.freeze_margin)
            .sum::<Decimal>();

        let position_equity: Decimal = inner
            .positions
            .values()
            .map(|v| v.margin + v.profit)
            .sum();

        Ok(inner.cash + position_equity + pending_freeze_margin)
    }

    async fn get_cash(&self) -> anyhow::Result<Decimal> {
        Ok(self.inner.lock().await.cash)
    }

    async fn get_leverage(&self, symbol: &str) -> anyhow::Result<u32> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_leverage: {}", symbol))?;

        Ok(self.inner.lock().await.leverage(symbol))
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("set_leverage: {}", symbol))?;

        if leverage == 0 {
            bail!("set_leverage: {}: leverage must be greater than 0", symbol);
        }

        let mut inner = self.inner.lock().await;
        let metadata = inner.metadata(symbol).cloned().expect("metadata not found");

        if inner
            .pending_order_list
            .iter()
            .any(|v| v.1.symbol == symbol && v.1.kind.is_normal())
        {
            bail!(
                "set_leverage: {}: there are currently pending orders, unable to modify the leverage",
                symbol
            );
        }

        let (append_margin, new_margin) = if let Some(v) = inner.positions.get(symbol) {
            let new_margin =
                calc_initial_margin(v.open_avg_price, v.quantity, leverage);
            (new_margin - v.margin, new_margin)
        } else {
            (Decimal::ZERO, Decimal::ZERO)
        };

        if append_margin > Decimal::ZERO && inner.cash < append_margin {
            bail!(
                "set_leverage: {}: cash shortage, adjusting leverage requires additional margin: {}",
                symbol,
                append_margin
            );
        }

        inner.leverages.insert(symbol.to_string(), leverage);
        inner.cash -= append_margin;

        let liquidation_update = if let Some(v) = inner.positions.get_mut(symbol) {
            v.leverage = leverage;
            v.margin = new_margin;
            v.liquidation_price = calc_liquidation_price(
                v.leverage,
                metadata.maintenance,
                v.side,
                v.open_avg_price,
                v.quantity,
                v.margin,
            );

            Some((v.liquidation_order_id.clone(), v.liquidation_price))
        } else {
            None
        };

        if let Some((liquidation_order_id, liquidation_price)) = liquidation_update
            && let Some(liquidation_order) =
                inner.pending_order_list.get_mut(&liquidation_order_id)
        {
            liquidation_order.price = liquidation_price;
        }

        Ok(())
    }

    async fn get_metadata(&self, symbol: &str) -> anyhow::Result<Metadata> {
        let inner = self.inner.lock().await;

        match inner.metadata(symbol) {
            Some(metadata) => Ok(metadata.clone()),
            None => bail!("get_metadata: no symbol: {}", symbol),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const BTC: &str = "BTCUSDT";
    const ETH: &str = "ETHUSDT";

    fn gen_kline(time: u64, open: Decimal, high: Decimal, low: Decimal, close: Decimal) -> KLine {
        KLine {
            time,
            open,
            high,
            low,
            close,
            volume: Decimal::ONE,
        }
    }

    fn btc_metadata() -> Metadata {
        Metadata {
            symbol: BTC.to_string(),
            level: Level::Minute1,
            min_size: dec!(0.001),
            min_notional: dec!(0.0),
            tick_size: dec!(0.1),
            maker_fee: dec!(0.0002),
            taker_fee: dec!(0.0005),
            maintenance: dec!(0.004),
        }
    }

    fn eth_metadata() -> Metadata {
        Metadata {
            symbol: ETH.to_string(),
            level: Level::Minute1,
            min_size: dec!(0.001),
            min_notional: dec!(0.0),
            tick_size: dec!(0.01),
            maker_fee: dec!(0.0002),
            taker_fee: dec!(0.0005),
            maintenance: dec!(0.004),
        }
    }

    fn btc_klines() -> Vec<KLine> {
        vec![
            gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.5)),
            gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.5)),
            gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.5)),
        ]
    }

    fn eth_klines() -> Vec<KLine> {
        vec![
            gen_kline(1, dec!(50.0), dec!(51.0), dec!(49.0), dec!(50.5)),
            gen_kline(2, dec!(52.0), dec!(53.0), dec!(51.0), dec!(52.5)),
            gen_kline(3, dec!(55.0), dec!(56.0), dec!(54.0), dec!(55.5)),
        ]
    }

    fn multi_exchange() -> ExchangeWrapper {
        ExchangeWrapper::new(Arc::new(
            LocalExchangeEx::new(vec![
                DataSource::new(btc_metadata(), btc_klines()),
                DataSource::new(eth_metadata(), eth_klines()),
            ])
            .cash(10000.0)
            .leverage(10),
        ))
    }

    // ---- multi-symbol round-trip ----

    #[tokio::test]
    async fn multi_symbol_round_advances_together() {
        let exchange = multi_exchange();

        // Round 1: both symbols at index 0
        let btc1 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth1 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc1.time, 1);
        assert_eq!(eth1.time, 1);

        // Round 2: both symbols at index 1
        let btc2 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth2 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc2.time, 2);
        assert_eq!(eth2.time, 2);

        // Round 3: both symbols at index 2
        let btc3 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth3 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc3.time, 3);
        assert_eq!(eth3.time, 3);

        // Exhausted
        assert!(exchange.next(BTC, Level::Minute1).await.unwrap().is_none());
        assert!(exchange.next(ETH, Level::Minute1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn multi_symbol_independent_positions() {
        let exchange = multi_exchange();

        // Round 1: place orders
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.sell(ETH, 1.0).await.unwrap();

        // Round 2: orders fill
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();

        assert_eq!(btc_pos.side, Side::Buy);
        assert_eq!(btc_pos.quantity, dec!(1.0));
        assert_eq!(btc_pos.open_avg_price, dec!(105.0));

        assert_eq!(eth_pos.side, Side::Sell);
        assert_eq!(eth_pos.quantity, dec!(1.0));
        assert_eq!(eth_pos.open_avg_price, dec!(52.0));
    }

    #[tokio::test]
    async fn multi_symbol_equity_sums_all_positions() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let equity = exchange.get_equity().await.unwrap();
        // Both positions exist, equity = cash + sum(margin + profit)
        assert!(equity > dec!(0.0));
    }

    #[tokio::test]
    async fn multi_symbol_close_all_position() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.sell(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_some());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());

        exchange.close_all_position(BTC).await.unwrap();
        exchange.close_all_position(ETH).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(exchange.get_position(ETH).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn multi_symbol_per_symbol_leverage() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        // Default leverage is 10
        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 10);
        assert_eq!(exchange.get_leverage(ETH).await.unwrap(), 10);

        // Change only BTC leverage
        exchange.set_leverage(BTC, 20).await.unwrap();
        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 20);
        assert_eq!(exchange.get_leverage(ETH).await.unwrap(), 10);
    }

    #[tokio::test]
    async fn multi_symbol_unknown_symbol_rejected() {
        let exchange = multi_exchange();

        let err = exchange.next("SOLUSDT", Level::Minute1).await.unwrap_err();
        assert!(err.to_string().contains("unknown symbol"));
    }

    #[tokio::test]
    async fn multi_symbol_metadata_per_symbol() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_meta = exchange.get_metadata(BTC).await.unwrap();
        let eth_meta = exchange.get_metadata(ETH).await.unwrap();

        assert_eq!(btc_meta.tick_size, dec!(0.1));
        assert_eq!(eth_meta.tick_size, dec!(0.01));
    }

    #[tokio::test]
    async fn multi_symbol_range_filters_all() {
        let exchange = LocalExchangeEx::new(vec![
            DataSource::new(btc_metadata(), btc_klines()),
            DataSource::new(eth_metadata(), eth_klines()),
        ])
        .cash(10000.0)
        .leverage(10)
        .range(1, 2); // times 1 and 2 (inclusive)

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        // Round 1
        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 1);
        assert_eq!(eth.time, 1);

        // Round 2
        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 2);
        assert_eq!(eth.time, 2);

        // Exhausted
        assert!(exchange.next(BTC, Level::Minute1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn single_symbol_works_like_original() {
        // Backward compatibility: single DataSource should work identically
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchangeEx::new(vec![DataSource::new(btc_metadata(), btc_klines())])
                .cash(10000.0)
                .leverage(10),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, dec!(105.0));
        assert_eq!(position.open_avg_price, dec!(105.0));
        assert_eq!(position.quantity, dec!(1.0));
    }
}
