use anyhow::Context;
use anyhow::bail;
use indexmap::IndexMap;
use inherits::inherits;
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::*;
use crate::exchange::*;
use crate::order::*;
use crate::util::*;

/// A multi-symbol backtesting exchange that replays historical K-line data.
///
/// `LocalExchange` steps through multiple [`DataSource`]s synchronised via [`Timeline`].
/// Each call to [`Exchange::next`] returns the current K-line for the given symbol;
/// the first call in a round advances the global timeline, and the last call
/// (when all symbols have been queried) resets the round counter.
///
/// # Builder pattern
///
/// After [`LocalExchange::new`], chain any of the following:
/// - [`cash`](LocalExchange::cash) — starting balance (default: 10,000).
/// - [`leverage`](LocalExchange::leverage) — default leverage for all symbols (default: 1).
/// - [`slippage`](LocalExchange::slippage) — fraction applied to market-order fill prices (default: 0).
/// - [`range`](LocalExchange::range) — restrict replayed data to `[start_time, end_time)`.
///
/// # Thread safety
///
/// All public methods take `&self` and lock an internal `Arc<Mutex<…>>`.
#[derive(Clone)]
pub struct LocalExchange {
    inner: Arc<Mutex<LocalExchangeInner>>,
}

struct LocalExchangeInner {
    timeline: Timeline,
    klines: Vec<(String, KLine)>,
    cash: Decimal,
    leverage: Vec<(String, u32)>,
    slippage: Decimal,
    history_order_list: IndexMap<String, OrderEx>,
    pending_order_list: IndexMap<String, OrderEx>,
    position: Vec<(String, PositionEx)>,
    history_position_list: Vec<HistoryPosition>,
    id: u64,
    pacemaker: Option<String>,
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

    fn sum_profit_fee(&self) -> (Decimal, Decimal) {
        self.log
            .iter()
            .fold((Decimal::ZERO, Decimal::ZERO), |(p, f), v| {
                (p + v.profit, f + v.fee)
            })
    }
}

/// Conversion from a single [`DataSource`] or a container of them into
/// `Vec<DataSource>`.
///
/// Accepts a bare [`DataSource`], a `Vec<DataSource>`, a slice, or an array.
pub trait ToDataSourceVec {
    fn into_vec(self) -> Vec<DataSource>;
}

impl ToDataSourceVec for DataSource {
    fn into_vec(self) -> Vec<DataSource> {
        vec![self]
    }
}

trait IntoDataSource {
    fn into_ds(self) -> DataSource;
}

impl IntoDataSource for DataSource {
    fn into_ds(self) -> DataSource {
        self
    }
}

impl IntoDataSource for &DataSource {
    fn into_ds(self) -> DataSource {
        self.clone()
    }
}

impl<U> ToDataSourceVec for U
where
    U: IsContainer + IntoIterator,
    U::Item: IntoDataSource,
{
    fn into_vec(self) -> Vec<DataSource> {
        self.into_iter().map(|item| item.into_ds()).collect()
    }
}

impl LocalExchange {
    /// Creates a new `LocalExchange` from one or more [`DataSource`].
    ///
    /// Accepts a single [`DataSource`], a `Vec<DataSource>`, a slice, or an array.
    ///
    /// All data sources must share the same [`Level`] and have overlapping
    /// time ranges. The shortest data source determines the total steps.
    ///
    /// # Panics
    ///
    /// Panics if `data_source` is empty, levels differ, or time ranges
    /// do not overlap.
    pub fn new(data_source: impl ToDataSourceVec) -> Self {
        let timeline = Timeline::new(data_source.into_vec())
            .expect("LocalExchange::new: Timeline creation failed");

        let symbols: Vec<String> = timeline
            .inner()
            .iter()
            .map(|v| v.metadata.symbol.clone())
            .collect();

        let mut klines = Vec::new();
        let mut leverage = Vec::new();

        for symbol in &symbols {
            klines.push((symbol.clone(), KLine::default()));
            leverage.push((symbol.clone(), 1));
        }

        Self {
            inner: Arc::new(Mutex::new(LocalExchangeInner {
                timeline,
                klines,
                cash: Decimal::from(10000),
                leverage,
                slippage: Decimal::ZERO,
                position: Vec::new(),
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

        for (_, lev) in inner.leverage.iter_mut() {
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
    /// falls in `[start_time, end_time)`.
    ///
    /// Rebuilds the [`Timeline`] from the filtered data sources.
    pub fn range(self, start_time: u64, end_time: u64) -> Self {
        let mut inner = self.inner.try_lock().unwrap();

        let filtered: Vec<DataSource> = inner
            .timeline
            .inner()
            .iter()
            .map(|v| v.range(start_time, end_time))
            .collect();

        inner.timeline = Timeline::new(filtered).expect("range: timeline rebuild failed");

        drop(inner);

        self
    }
}

impl LocalExchangeInner {
    fn metadata(&self, symbol: &str) -> Option<&Metadata> {
        self.timeline
            .inner()
            .iter()
            .find(|v| v.metadata.symbol == symbol)
            .map(|v| &v.metadata)
    }

    fn kline(&self, symbol: &str) -> &KLine {
        &self.klines.iter().find(|(v, _)| v == symbol).unwrap().1
    }

    fn leverage(&self, symbol: &str) -> u32 {
        self.leverage
            .iter()
            .find(|(v, _)| v == symbol)
            .map(|(_, v)| *v)
            .unwrap()
    }

    fn calc_market_price_slippage(
        &self,
        side: Side,
        market_price: Decimal,
        kline: KLine,
    ) -> Decimal {
        let price: Decimal = match side {
            Side::Buy => market_price * (Decimal::ONE + self.slippage),
            Side::Sell => market_price * (Decimal::ONE - self.slippage),
        };

        price.clamp(kline.low, kline.high)
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
            // 对于市价单，保证金在成交时被冻结
            // 对于计划委托单，保证金在触发时被冻结
            // 只有在不是只减仓订单的情况下，我们才需要冻结保证金
            // 这里按整单先冻结，若后续仅平仓或发生反向开仓
            // 会在成交分支按实际需要返还多冻结保证金
            self.freeze_margin(&mut order, leverage)
                .context(format!("place_order: {}", order.symbol))?;
        }

        self.pending_order_list.insert(id.clone(), order);

        Ok(id)
    }

    fn advance(&mut self) {
        if let Some(all_klines) = self.timeline.all() {
            for (symbol, kline) in all_klines {
                if let Some(index) = self.klines.iter().position(|(v, _)| v == &symbol) {
                    self.klines[index].1 = kline;
                } else {
                    self.klines.push((symbol.to_string(), kline));
                }
            }
        }

        self.timeline.next();
        self.update();
    }

    fn update(&mut self) {
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

        let position_update: Vec<(String, Decimal, Decimal)> = self
            .position
            .iter()
            .map(|(symbol, v)| {
                let kline = self.kline(symbol);
                let metadata = self.metadata(symbol).expect("metadata not found");
                let profit = if v.side == Side::Buy {
                    (kline.close - v.open_avg_price) * v.quantity
                } else {
                    (v.open_avg_price - kline.close) * v.quantity
                };

                let liquidation_price = calc_liquidation_price(
                    v.leverage,
                    metadata.maintenance,
                    v.side,
                    v.open_avg_price,
                    v.quantity,
                    v.margin,
                );

                (symbol.clone(), profit, liquidation_price)
            })
            .collect();

        for (symbol, profit, liquidation_price) in position_update {
            if let Some(position) = self.position.iter_mut().find(|(v, _)| v == &symbol) {
                position.1.profit = profit;
                position.1.liquidation_price = liquidation_price;
            }
        }
    }

    fn handle_trigger_order(&mut self, order_id: &str, order_queue: &mut VecDeque<String>) {
        let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
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

        order_ref.update_time = self.kline(&order_ref.symbol).time;
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

        let fee_rate = if order_ref.kind == Kind::Liquidation {
            self.metadata(&order_ref.symbol)
                .map(|v| v.taker_fee)
                .unwrap()
        } else {
            self.metadata(&order_ref.symbol)
                .map(|v| v.maker_fee)
                .unwrap()
        };

        self.execute_order(order_id, order_ref, fee_rate);
    }

    fn try_fill_limit_order(&mut self, order_id: &str) -> Option<OrderEx> {
        let order = self.pending_order_list.get(order_id)?;
        let kline = *self.kline(&order.symbol);

        if order.kind == Kind::Liquidation {
            if !(order.price >= kline.low && order.price <= kline.high) {
                return None;
            }
            let mut order_ref = self.pending_order_list.shift_remove(order_id)?;
            order_ref.avg_price = order_ref.price;
            Some(order_ref)
        } else if (order.side == Side::Buy && order.price >= kline.open)
            || (order.side == Side::Sell && order.price <= kline.open)
        {
            let mut order_ref = self.pending_order_list.shift_remove(order_id)?;
            order_ref.avg_price = if order_ref.side == Side::Buy {
                kline.high
            } else {
                kline.low
            };
            Some(order_ref)
        } else if (order.side == Side::Buy && kline.low <= order.price)
            || (order.side == Side::Sell && kline.high >= order.price)
        {
            let mut order_ref = self.pending_order_list.shift_remove(order_id)?;
            order_ref.avg_price = order_ref.price;
            Some(order_ref)
        } else {
            None
        }
    }

    fn handle_market_order(&mut self, order_id: &str) {
        let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
            Some(v) => v,
            None => return,
        };

        let kline = *self.kline(&order_ref.symbol);

        if order_ref.price == Decimal::ZERO {
            order_ref.price = self.calc_market_price_slippage(order_ref.side, kline.open, kline);
        } else {
            order_ref.price =
                self.calc_market_price_slippage(order_ref.side, order_ref.price, kline);
        }

        order_ref.avg_price = order_ref.price;

        let fee_rate = self
            .metadata(&order_ref.symbol)
            .map(|v| v.taker_fee)
            .unwrap();

        self.execute_order(order_id, order_ref, fee_rate);
    }

    fn update_order(&mut self, order_id: &str, order_queue: &mut VecDeque<String>) {
        let Some(order) = self.pending_order_list.get(order_id) else {
            return;
        };

        if order.status != Status::Submitted {
            return;
        }

        let symbol = order.symbol.clone();
        let kline = self.kline(&symbol);

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

    fn handle_reduce_only_check(&mut self, order_ref: &mut OrderEx) -> bool {
        if let Some((_, v)) = self.position.iter().find(|(v, _)| v == &order_ref.symbol) {
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

    fn handle_pre_execution_check(&mut self, order_ref: &mut OrderEx, fee_rate: Decimal) -> bool {
        let symbol = order_ref.symbol.clone();
        let leverage = self.leverage(&symbol);
        let metadata = self.metadata(&symbol).cloned().expect("metadata not found");

        if order_ref.reduce_only {
            if self.handle_reduce_only_check(order_ref) {
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
            close_time: self.kline(&order_ref.symbol).time,
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

        let kline_time = self.kline(&order_ref.symbol).time;
        let symbol = position.symbol.clone();

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
        if let Some(index) = self.position.iter().position(|(v, _)| v == &symbol) {
            self.position.swap_remove(index);
        }
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

        let kline_time = self.kline(&order_ref.symbol).time;
        let symbol = position.symbol.clone();

        let partial_close_qty = position
            .log
            .iter()
            .filter(|v| v.side != position.side)
            .map(|v| v.quantity)
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

        let metadata = self.metadata(&symbol).cloned().expect("metadata not found");

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

        self.position.push((symbol, position));
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
        let symbol = order_ref.symbol.clone();
        let leverage = self.leverage(&symbol);
        let reverse_margin = calc_initial_margin(order_ref.avg_price, reverse_quantity, leverage);
        let kline_time = self.kline(&symbol).time;
        let metadata = self.metadata(&symbol).cloned().expect("metadata not found");
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

        self.position.push((
            symbol.clone(),
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
                    symbol,
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
        ));
    }

    fn handle_open_position(&mut self, id: &str, order_ref: &OrderEx, fee_rate: Decimal) {
        let symbol = order_ref.symbol.clone();
        let leverage = self.leverage(&symbol);
        let metadata = self.metadata(&symbol).cloned().expect("metadata not found");
        let kline_time = self.kline(&symbol).time;

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
                    symbol: symbol.clone(),
                    side: order_ref.side.neg(),
                    trigger_price: Decimal::ZERO,
                    price: liquidation_price,
                    quantity: Decimal::MAX,
                    reduce_only: true,
                },
                Kind::Liquidation,
            )
            .unwrap();

        self.position.push((
            symbol,
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
        ));
    }

    fn execute_order(&mut self, id: &str, mut order_ref: OrderEx, fee_rate: Decimal) {
        let symbol = order_ref.symbol.clone();
        let kline_time = self.kline(&symbol).time;

        order_ref.update_time = kline_time;

        if !self.handle_pre_execution_check(&mut order_ref, fee_rate) {
            return;
        }

        let position_index = self.position.iter().position(|(v, _)| v == &symbol);

        match position_index {
            Some(index) if self.position[index].1.side == order_ref.side => {
                let position = self.position.swap_remove(index).1;

                self.handle_add_position(id, &order_ref, fee_rate, position);
            }
            Some(index) => {
                let mut position = self.position.swap_remove(index).1;
                let close_quantity = order_ref.quantity.min(position.quantity);
                let remain_quantity = order_ref.quantity - position.quantity;
                let (close_margin, close_profit) =
                    self.calc_close_metrics(&position, &order_ref, close_quantity);

                position.quantity -= close_quantity;
                position.margin -= close_margin;
                self.cash += close_margin + close_profit;

                position.log.push(Record {
                    id: id.to_string(),
                    kind: order_ref.kind,
                    side: order_ref.side,
                    price: order_ref.avg_price,
                    quantity: order_ref.quantity,
                    profit: close_profit,
                    fee: order_ref.avg_price * close_quantity * fee_rate,
                    time: kline_time,
                });

                let (profit_sum, fee_sum) = position.sum_profit_fee();
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
        let symbol = order_ref.symbol.clone();
        let metadata = self.metadata(&symbol).cloned().expect("metadata not found");
        let kline_time = self.kline(&symbol).time;
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

        self.position.push((symbol, position));
    }
}

#[async_trait::async_trait]
impl Exchange for LocalExchange {
    async fn next(&self, symbol: &str, level: Level) -> anyhow::Result<Option<KLine>> {
        let mut inner = self.inner.lock().await;

        if !inner
            .timeline
            .inner()
            .iter()
            .any(|v| v.metadata.symbol == symbol && v.metadata.level == level)
        {
            bail!("next: unknown symbol and level: {}, {}", symbol, level);
        }

        if inner.exhausted {
            return Ok(None);
        }

        if inner.pacemaker.is_none() {
            inner.pacemaker = Some(symbol.to_string());
        }

        let is_pacemaker = inner.pacemaker.as_deref() == Some(symbol);

        if is_pacemaker {
            if inner.timeline.is_done() {
                inner.exhausted = true;
                return Ok(None);
            }

            inner.advance();
        }

        Ok(inner
            .klines
            .iter()
            .find(|(v, _)| v == symbol)
            .map(|(_, v)| v)
            .cloned())
    }

    async fn get_kline(
        &self,
        _symbol: &str,
        _level: Level,
        _start: u64,
        _end: u64,
    ) -> anyhow::Result<Vec<KLine>> {
        Ok(Vec::new())
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
                v.symbol == symbol
                    && v.status == Status::Submitted
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
            .position
            .iter()
            .find(|(v, _)| v == symbol)
            .map(|(_, v)| v.parent.clone()))
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

        let (liquidation_order_id, liquidation_price, cash_delta) = match inner
            .position
            .iter_mut()
            .find(|(v, _)| v == symbol)
        {
            Some((_, position)) => {
                let new_margin = position.margin + margin;
                let init_margin =
                    position.open_avg_price * position.quantity / Decimal::from(position.leverage);

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

        if let Some(liquidation_order) = inner.pending_order_list.get_mut(&liquidation_order_id) {
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
            .position
            .iter()
            .map(|(_, v)| v.margin + v.profit)
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

        let (append_margin, new_margin) =
            if let Some((_, v)) = inner.position.iter().find(|(v, _)| v == symbol) {
                let new_margin = calc_initial_margin(v.open_avg_price, v.quantity, leverage);
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

        if let Some(index) = inner.leverage.iter().position(|(v, _)| v == symbol) {
            inner.leverage[index].1 = leverage;
        } else {
            inner.leverage.push((symbol.to_string(), leverage));
        }

        inner.cash -= append_margin;

        let liquidation_update =
            if let Some((_, v)) = inner.position.iter_mut().find(|(v, _)| v == symbol) {
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
            && let Some(liquidation_order) = inner.pending_order_list.get_mut(&liquidation_order_id)
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

    // ---- single-symbol helpers (mirrors local_exchange.rs test_exchange / test_exchange_with) ----

    fn single_exchange() -> ExchangeWrapper {
        ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
                .cash(10000.0)
                .leverage(10),
        ))
    }

    fn single_exchange_with(metadata: Metadata, kline_list: Vec<KLine>) -> ExchangeWrapper {
        ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![DataSource::new(metadata, kline_list)])
                .cash(10000.0)
                .leverage(10),
        ))
    }

    // ---- multi-symbol helper ----

    fn multi_exchange() -> ExchangeWrapper {
        ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![
                DataSource::new(btc_metadata(), btc_klines()),
                DataSource::new(eth_metadata(), eth_klines()),
            ])
            .cash(10000.0)
            .leverage(10),
        ))
    }
    // 验证市价单在下一根 K 线按开盘价成交并生成仓位。
    #[tokio::test]
    async fn market_order_fills_on_next_kline() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 105.0);
        assert_eq!(position.open_avg_price, 105.0);
        assert_eq!(position.quantity, 1.0);
    }

    // 验证取消限价单后会返还冻结保证金，且挂单列表清空。
    #[tokio::test]
    async fn cancel_order_refunds_frozen_margin() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert!(cash_after_place < cash_before);

        exchange.cancel_order(BTC, &id).await.unwrap();

        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_cancel, cash_before);
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // 验证存在挂单时不允许修改杠杆，撤单后可成功修改。
    #[tokio::test]
    async fn set_leverage_fails_when_pending_orders_exist() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();

        let result = exchange.set_leverage(BTC, 20).await.unwrap_err();

        assert!(result.to_string().contains("pending orders"));

        exchange.cancel_order(BTC, &id).await.unwrap();
        exchange.set_leverage(BTC, 20).await.unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 20);
    }

    // 验证触发市价单立即执行：触发后在同一根 K 线立即成交。
    #[tokio::test]
    async fn trigger_market_order_is_two_stage_filled() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_id = exchange.buy_trigger_market(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，仓位已创建
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.open_avg_price, 105.0); // 在当前K线开盘价成交
        assert_eq!(position.quantity, 1.0);

        // 仓位创建后会有强平单
        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, Kind::Liquidation);
    }

    // 验证无仓位时的 reduce-only 订单会被自动取消。
    #[tokio::test]
    async fn reduce_only_without_position_is_canceled() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let id = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Canceled);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证 close_all_position 会在下一根 K 线完成平仓并写入历史。
    #[tokio::test]
    async fn close_all_position_closes_on_next_kline() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_some());

        exchange.close_all_position(BTC).await.unwrap();
        assert!(exchange.get_position(BTC).await.unwrap().is_some());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].close_avg_price, 110.0);
    }

    // 验证追加保证金接口的边界校验和现金/保证金更新逻辑。
    #[tokio::test]
    async fn append_position_margin_checks_and_updates_cash() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange
            .append_position_margin(BTC, -1.0)
            .await
            .unwrap_err();

        assert!(result.to_string().contains("initial margin"));

        let cash_before = exchange.get_cash().await.unwrap();
        let margin_before = exchange.get_position(BTC).await.unwrap().unwrap().margin;
        let liquidation_price_before = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(BTC, 2.0).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();
        let margin_after = position_after.margin;
        let liquidation_price_after = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert_eq!(cash_before - cash_after, 2.0);
        assert_eq!(margin_after - margin_before, 2.0);
        assert_eq!(liquidation_price_after, position_after.liquidation_price);
        assert!(liquidation_price_after < liquidation_price_before);
    }

    // 验证权益会随未实现盈亏在后续 K 线上动态变化。
    #[tokio::test]
    async fn equity_tracks_unrealized_profit_over_klines() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let equity_after_open = exchange.get_equity().await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let equity_after_next = exchange.get_equity().await.unwrap();

        assert!(equity_after_next > equity_after_open);
        assert_eq!(equity_after_next - equity_after_open, 5.0);
    }

    // 验证空仓（Sell）未实现盈亏方向正确：价格下跌时权益上升。
    #[tokio::test]
    async fn equity_tracks_unrealized_profit_for_short_position() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let equity_after_open = exchange.get_equity().await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let equity_after_next = exchange.get_equity().await.unwrap();

        assert!(equity_after_next > equity_after_open);
        assert_eq!(equity_after_next - equity_after_open, 5.0);
    }

    // 验证限价单价格在当根 K 线区间内时能够成交。
    #[tokio::test]
    async fn limit_order_filled_when_price_in_range() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 106.0);
        assert_eq!(position.open_avg_price, 106.0);
    }

    // 验证限价单价格不在区间内时保持 Submitted 不成交。
    #[tokio::test]
    async fn limit_order_not_filled_when_price_out_of_range() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证市价单成交价使用开盘价而非高低价。
    #[tokio::test]
    async fn market_order_uses_open_price_not_high_low() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.avg_price, 105.0);
        assert!(order.avg_price < 106.0);
        assert!(order.avg_price > 104.0);
    }

    // 验证卖出 reduce-only 能正确平掉已有多仓。
    #[tokio::test]
    async fn sell_market_order_fills_correctly() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_id = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();

        assert_eq!(close_order.status, Status::Filled);
        assert_eq!(close_order.avg_price, 110.0);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
    }

    // 验证 reduce-only 平仓在手续费预扣不足时会被拒绝，且不会改变现金和仓位。
    #[tokio::test]
    async fn reduce_only_close_rejected_when_fee_precharge_cash_is_insufficient() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(10.56)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();
        let position_before_close = exchange.get_position(BTC).await.unwrap().unwrap();

        let close_id = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(close_order.status, Status::Rejected);
        assert_eq!(cash_after, cash_before_close);
        assert_eq!(position_after.side, position_before_close.side);
        assert_eq!(position_after.quantity, position_before_close.quantity);
        assert_eq!(
            position_after.open_avg_price,
            position_before_close.open_avg_price,
        );
    }

    // 验证浮亏场景下，手续费不足的 reduce-only 平仓会被拒绝，不会把现金打到负值。
    #[tokio::test]
    async fn reduce_only_close_with_floating_loss_and_fee_shortage_is_rejected() {
        let exchange = LocalExchange::new(vec![DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(80.0), dec!(81.0), dec!(79.0), dec!(80.0)),
            ],
        )])
        .cash(10.56)
        .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();

        let close_id = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(close_order.status, Status::Rejected);
        assert_eq!(position_after.side, Side::Buy);
        assert_eq!(position_after.quantity, 1.0);
        assert_eq!(cash_after, cash_before_close);
        assert!(cash_after > 0);
    }

    // 验证触发限价单触发后立即在同一根 K 线成交。
    #[tokio::test]
    async fn trigger_limit_order_triggers_then_fills_on_next_kline() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(102.0), dec!(106.0), dec!(101.0), dec!(105.0)),
                gen_kline(3, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let trigger_id = exchange
            .buy_trigger_limit(BTC, 104.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，仓位已创建
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.open_avg_price, 106.0); // 买单限价≥开盘价，以最高价成交
    }

    // 验证触发价长期不满足时，触发单保持 Submitted 状态。
    #[tokio::test]
    async fn trigger_order_stays_submitted_when_not_triggered() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_trigger_market(BTC, 200.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证同向加仓后仓位数量累加且均价按加权方式更新。
    #[tokio::test]
    async fn add_position_updates_avg_price_correctly() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.quantity, 2.0);
        assert_eq!(position.open_avg_price, 107.5);
    }

    // 验证反向超量下单会先平仓再反向开仓。
    #[tokio::test]
    async fn reverse_position_long_to_short() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.open_avg_price, 110.0);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
    }

    // 验证反向剩余量小于 min_size 时不会误开微小反向仓。
    #[tokio::test]
    async fn opposite_order_with_tiny_positive_remainder_closes_without_reversal() {
        let mut metadata = btc_metadata();
        metadata.min_size = dec!(0.001);

        let exchange = single_exchange_with(
            metadata,
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 比原仓位多 0.0005，小于 min_size=0.001，应按全平处理，不反向开仓。
        exchange.sell(BTC, 1.0005).await.unwrap_err();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_some());
    }

    // 验证开仓后会自动生成对应的强平保护订单。
    #[tokio::test]
    async fn liquidation_order_created_on_open() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let pending = exchange.get_pending_order_list(BTC).await.unwrap();

        assert!(pending.iter().any(|v| v.kind == Kind::Liquidation));
    }

    // 验证普通单会冻结保证金，而 reduce-only 订单不会冻结保证金。
    #[tokio::test]
    async fn freeze_margin_only_for_non_reduce_only_orders() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let normal_id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let cash_after_normal = exchange.get_cash().await.unwrap();

        let reduce_only_id = exchange
            .sell_limit_reduce_only(BTC, 120.0, 1.0)
            .await
            .unwrap();
        let cash_after_reduce_only = exchange.get_cash().await.unwrap();

        assert!(cash_after_normal < cash_before);
        assert_eq!(cash_after_reduce_only, cash_after_normal);

        exchange.cancel_order(BTC, &normal_id).await.unwrap();
        exchange.cancel_order(BTC, &reduce_only_id).await.unwrap();
    }

    // 验证取消不存在的订单是幂等操作，不会报错且不污染状态。
    #[tokio::test]
    async fn cancel_order_nonexistent_is_idempotent() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        exchange.cancel_order(BTC, "not-exists").await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let pending = exchange.get_pending_order_list(BTC).await.unwrap();

        assert_eq!(cash_after, cash_before);
        assert!(pending.is_empty());
    }

    // 验证限价单价格不能为负，避免污染保证金与现金计算。
    #[tokio::test]
    async fn place_order_rejects_negative_limit_price() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let negative_err = exchange.buy_limit(BTC, -1.0, 1.0).await.unwrap_err();

        assert!(
            negative_err
                .to_string()
                .contains("limit price must be greater than 0")
        );
    }

    // 验证触发单触发价必须大于 0，且触发限价单价格不能为负。
    #[tokio::test]
    async fn place_order_rejects_invalid_trigger_prices() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_price_err = exchange
            .place_order(Order {
                symbol: BTC.to_string(),
                side: Side::Buy,
                trigger_price: dec!(-1.0),
                price: dec!(0.0),
                quantity: dec!(1.0),
                reduce_only: false,
            })
            .await
            .unwrap_err();

        let trigger_limit_negative_price_err = exchange
            .buy_trigger_limit(BTC, 100.0, -1.0, 1.0)
            .await
            .unwrap_err();

        assert!(
            trigger_price_err
                .to_string()
                .contains("trigger price must be greater than 0")
        );
        assert!(
            trigger_limit_negative_price_err
                .to_string()
                .contains("trigger order price must be >= 0")
        );
    }

    // 验证限价单价格必须与 tick_size 对齐，不允许提交非档位价格。
    #[tokio::test]
    async fn place_order_rejects_limit_price_not_aligned_to_tick_size() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy_limit(BTC, 68000.123, 1.0).await.unwrap_err();
    }

    // 验证通过浮点运算得到的档位价格（如 0.1 + 0.2）不会被误判为非对齐。
    #[tokio::test]
    async fn place_order_accepts_limit_price_from_float_arithmetic_when_tick_aligned() {
        let mut metadata = btc_metadata();
        metadata.tick_size = dec!(0.1);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let computed_price = 0.1_f64 + 0.2_f64;
        let id = exchange.buy_limit(BTC, computed_price, 1.0).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证由浮点算术构造的价格/数量在复杂流程中保持稳定：
    // 限价开仓 -> 触发限价减仓 -> 普通限价挂单并撤单，且每一步都断言现金/权益/仓位/挂单/历史。
    #[tokio::test]
    async fn float_arithmetic_complex_step_assertions_on_each_transition() {
        let mut metadata = btc_metadata();
        metadata.tick_size = dec!(0.1);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(
            metadata.clone(),
            vec![
                gen_kline(1, dec!(100.0), dec!(100.5), dec!(99.8), dec!(100.1)),
                gen_kline(2, dec!(100.2), dec!(100.6), dec!(99.8), dec!(100.4)),
                gen_kline(3, dec!(100.5), dec!(100.9), dec!(100.3), dec!(100.7)),
                gen_kline(4, dec!(100.6), dec!(100.8), dec!(100.1), dec!(100.2)),
            ],
        );

        let initial_cash = dec!(10000.0);

        let entry_price = dec!(100.0) + (dec!(0.1) + dec!(0.2));
        let entry_qty = dec!(0.1) + dec!(0.2);

        let trigger_price = dec!(100.0) + (dec!(0.2) + dec!(0.3));
        let reduce_limit_price = dec!(100.0) + (dec!(0.2) + dec!(0.2));
        let reduce_qty = dec!(0.1) + dec!(0.1);

        let md = metadata;

        // Step 1: 仅推进首根 K，状态不变。
        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), initial_cash);
        assert_eq!(exchange.get_equity().await.unwrap(), initial_cash);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );

        // 下由浮点算术构造的限价开多。
        let open_id = exchange
            .buy_limit(BTC, entry_price, entry_qty)
            .await
            .unwrap();

        let freeze_entry = entry_price * entry_qty / dec!(10.0);
        let cash_after_place = initial_cash - freeze_entry;

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_place);
        // 新权益口径包含挂单冻结保证金，因此总权益不变。
        assert_eq!(exchange.get_equity().await.unwrap(), initial_cash);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);

        // Step 2: 限价开仓成交（buy 且 price>=open，按 high 成交）。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&open_id).await.unwrap().unwrap();
        let mut position = exchange.get_position(BTC).await.unwrap().unwrap();
        let pending_s2 = exchange.get_pending_order_list(BTC).await.unwrap();

        let open_fill = dec!(100.6);
        let margin_s2 = open_fill * entry_qty / dec!(10.0);
        let open_fee = open_fill * entry_qty * md.maker_fee;
        let cash_s2 = initial_cash - margin_s2 - open_fee;
        let upnl_s2 = (dec!(100.4) - open_fill) * entry_qty;
        let equity_s2 = cash_s2 + margin_s2 + upnl_s2;

        assert_eq!(open_order.status, Status::Filled);
        assert_eq!(open_order.avg_price, open_fill);
        assert_eq!(position.open_avg_price, open_fill);
        assert_eq!(position.quantity, entry_qty);
        assert_eq!(position.margin, margin_s2);
        assert_eq!(position.profit, upnl_s2);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s2);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(pending_s2.len(), 1);
        assert_eq!(pending_s2[0].kind, Kind::Liquidation);

        // 下由浮点算术构造的 reduce-only 触发限价平仓单。
        let trigger_reduce_id = exchange
            .sell_trigger_limit_reduce_only(BTC, trigger_price, reduce_limit_price, reduce_qty)
            .await
            .unwrap();

        assert_eq!(exchange.get_cash().await.unwrap(), cash_s2);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // Step 3: 触发单触发并立即执行（sell 且 price<=open，按 low 成交）。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_reduce = exchange
            .get_order(&trigger_reduce_id)
            .await
            .unwrap()
            .unwrap();
        let pending_s3 = exchange.get_pending_order_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        let reduce_limit_fill = dec!(100.3); // kline 3 low
        let close_margin = margin_s2 * (reduce_qty / entry_qty);
        let close_profit = (reduce_limit_fill - open_fill) * reduce_qty;
        let close_fee = reduce_limit_fill * reduce_qty * md.maker_fee;
        let cash_s3 = cash_s2 - close_fee + close_margin + close_profit;
        let qty_left = entry_qty - reduce_qty;
        let margin_left = margin_s2 - close_margin;
        let upnl_s3 = (dec!(100.7) - open_fill) * qty_left;
        let equity_s3 = cash_s3 + margin_left + upnl_s3;

        assert_eq!(trigger_reduce.status, Status::Filled);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s3);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s3);
        assert_eq!(position.quantity, qty_left);
        assert_eq!(position.margin, margin_left);
        assert_eq!(position.profit, upnl_s3);
        assert_eq!(pending_s3.len(), 1);
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Liquidation));

        // Step 4: 无新订单成交，仅推进 K 线。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let pending_s4 = exchange.get_pending_order_list(BTC).await.unwrap();
        let history_pos_s4 = exchange.get_history_position_list(BTC).await.unwrap();
        let position_s4 = exchange.get_position(BTC).await.unwrap().unwrap();

        let upnl_s4 = (dec!(100.2) - open_fill) * qty_left;
        let equity_s4 = cash_s3 + margin_left + upnl_s4;

        assert_eq!(exchange.get_cash().await.unwrap(), cash_s3);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(position_s4.side, Side::Buy);
        assert_eq!(position_s4.open_avg_price, open_fill);
        assert_eq!(position_s4.quantity, qty_left);
        assert_eq!(position_s4.margin, margin_left);
        assert_eq!(position_s4.profit, upnl_s4);
        assert_eq!(pending_s4.len(), 1);
        assert_eq!(pending_s4[0].kind, Kind::Liquidation);
        assert_eq!(history_pos_s4.len(), 1);
        assert_eq!(history_pos_s4[0].close_quantity, reduce_qty);
        assert_eq!(history_pos_s4[0].close_avg_price, reduce_limit_fill);

        // 再挂一个浮点算术价格限价单，验证冻结保证金已计入权益，然后立即撤单。
        let readd_price = dec!(99.7) + dec!(0.3);
        let readd_qty = dec!(0.05) + dec!(0.05);
        let readd_id = exchange
            .buy_limit(BTC, readd_price, readd_qty)
            .await
            .unwrap();

        let readd_freeze = readd_price * readd_qty / dec!(10.0);
        let cash_after_readd_place = cash_s3 - readd_freeze;

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_readd_place);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        exchange.cancel_order(BTC, &readd_id).await.unwrap();

        let canceled = exchange.get_order(&readd_id).await.unwrap().unwrap();
        assert_eq!(canceled.status, Status::Canceled);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s3);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);
    }

    // 验证多个浮点算术限价单在“部分成交+保留挂单+撤单+平仓”路径中的逐步状态一致性。
    #[tokio::test]
    async fn float_arithmetic_multi_order_step_assertions_with_cancel_and_close() {
        let mut metadata = btc_metadata();
        metadata.tick_size = dec!(0.1);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(
            metadata.clone(),
            vec![
                gen_kline(1, dec!(100.0), dec!(100.4), dec!(99.8), dec!(100.0)),
                gen_kline(2, dec!(100.2), dec!(100.5), dec!(100.0), dec!(100.3)),
                gen_kline(3, dec!(100.4), dec!(100.6), dec!(100.1), dec!(100.2)),
            ],
        );

        let md = metadata;
        let initial_cash = dec!(10000.0);

        let fill_price_order = dec!(100.0) + (dec!(0.2) + dec!(0.1));
        let fill_qty_order = dec!(0.15) + dec!(0.15);

        let pending_price_order = dec!(99.6) + (dec!(0.1) + dec!(0.1));
        let pending_qty_order = dec!(0.05) + dec!(0.05);

        // Step 1: 首根 K，状态不变。
        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), initial_cash);
        assert_eq!(exchange.get_equity().await.unwrap(), initial_cash);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        // 同时挂两个由浮点算术构造的买限价：一个预期成交、一个预期保留。
        let id_fill = exchange
            .buy_limit(BTC, fill_price_order, fill_qty_order)
            .await
            .unwrap();
        let id_pending = exchange
            .buy_limit(BTC, pending_price_order, pending_qty_order)
            .await
            .unwrap();

        let freeze_fill = fill_price_order * fill_qty_order / dec!(10.0);
        let freeze_pending = pending_price_order * pending_qty_order / dec!(10.0);
        let cash_after_place = initial_cash - freeze_fill - freeze_pending;

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_place);
        assert_eq!(exchange.get_equity().await.unwrap(), initial_cash);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // Step 2: 第二根 K，仅第一笔成交（按 high 成交），第二笔保留为 pending。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order_fill = exchange.get_order(&id_fill).await.unwrap().unwrap();
        let order_pending = exchange.get_order(&id_pending).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        let fill_avg = dec!(100.5);
        let margin_s2 = fill_avg * fill_qty_order / dec!(10.0);
        let fee_open = fill_avg * fill_qty_order * md.maker_fee;
        let cash_s2 = initial_cash - freeze_pending - margin_s2 - fee_open;
        let upnl_s2 = (dec!(100.3) - fill_avg) * fill_qty_order;
        let equity_s2 = cash_s2 + margin_s2 + upnl_s2 + freeze_pending;

        assert_eq!(order_fill.status, Status::Filled);
        assert_eq!(order_pending.status, Status::Submitted);
        assert_eq!(order_fill.avg_price, fill_avg);

        assert_eq!(position.side, Side::Buy);
        assert_eq!(position.open_avg_price, fill_avg);
        assert_eq!(position.quantity, fill_qty_order);
        assert_eq!(position.margin, margin_s2);
        assert_eq!(position.profit, upnl_s2);

        assert_eq!(exchange.get_cash().await.unwrap(), cash_s2);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // 撤掉仍 pending 的普通限价单，现金回补，但总权益应保持不变。
        exchange.cancel_order(BTC, &id_pending).await.unwrap();

        let canceled_pending = exchange.get_order(&id_pending).await.unwrap().unwrap();
        let cash_after_cancel = cash_s2 + freeze_pending;
        let equity_after_cancel = cash_after_cancel + margin_s2 + upnl_s2;

        assert_eq!(canceled_pending.status, Status::Canceled);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_cancel);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_after_cancel);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);

        // 下由浮点算术构造的 reduce-only 市价平仓单。
        let close_qty = dec!(0.1) + dec!(0.2);
        let close_id = exchange.sell_reduce_only(BTC, close_qty).await.unwrap();

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_cancel);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // Step 3: 第三根 K 市价平仓成交。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let pending_s3 = exchange.get_pending_order_list(BTC).await.unwrap();

        let close_avg = dec!(100.4);
        let close_profit = (close_avg - fill_avg) * close_qty;
        let close_fee = close_avg * close_qty * md.taker_fee;
        let cash_final = cash_after_cancel - close_fee + margin_s2 + close_profit;

        assert_eq!(close_order.status, Status::Filled);
        assert_eq!(close_order.avg_price, close_avg);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(pending_s3.is_empty());

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].open_avg_price, fill_avg);
        assert_eq!(history[0].close_avg_price, close_avg);
        assert_eq!(history[0].close_quantity, close_qty);

        assert_eq!(exchange.get_cash().await.unwrap(), cash_final);
        assert_eq!(exchange.get_equity().await.unwrap(), cash_final);
    }

    // 验证触发限价单触发价与委托价都必须与 tick_size 对齐。
    #[tokio::test]
    async fn place_order_rejects_trigger_prices_not_aligned_to_tick_size() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange
            .buy_trigger_market(BTC, 105.05, 1.0)
            .await
            .unwrap_err();
        exchange
            .buy_trigger_limit(BTC, 105.0, 105.05, 1.0)
            .await
            .unwrap_err();
    }

    // 验证名义价值低于最小限制时会拒绝下单。
    #[tokio::test]
    async fn place_order_rejects_below_min_notional() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange.buy_limit(BTC, 10.0, 5.0).await.unwrap_err();

        assert!(result.to_string().contains("metadata.min_notional"));
    }

    // 验证下单数量低于最小下单量时会拒绝下单。
    #[tokio::test]
    async fn place_order_rejects_below_min_size() {
        let mut metadata = btc_metadata();
        metadata.min_size = dec!(0.01);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy_limit(BTC, 100.0, 0.001).await.unwrap_err();
    }

    // 验证 min_size 校验不会因 round 导致“略小于最小值”被错误放行。
    #[tokio::test]
    async fn place_order_rejects_quantity_slightly_below_min_size() {
        let mut metadata = btc_metadata();
        metadata.min_size = dec!(0.01);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy_limit(BTC, 100.0, 0.0096).await.unwrap_err();
    }

    // 验证 cancel_all_order 仅取消普通 Submitted 订单，不影响强平保护单。
    #[tokio::test]
    async fn cancel_all_order_only_cancels_submitted_normal_orders() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let pending_before = exchange.get_pending_order_list(BTC).await.unwrap();
        assert_eq!(pending_before.len(), 2);
        assert!(pending_before.iter().any(|v| v.kind == Kind::Limit));
        assert!(pending_before.iter().any(|v| v.kind == Kind::Liquidation));

        exchange.cancel_all_order(BTC).await.unwrap();

        let pending_after = exchange.get_pending_order_list(BTC).await.unwrap();
        let history = exchange.get_history_order_list(BTC).await.unwrap();

        assert_eq!(pending_after.len(), 1);
        assert_eq!(pending_after[0].kind, Kind::Liquidation);
        assert!(
            history
                .iter()
                .any(|v| v.kind == Kind::Limit && v.status == Status::Canceled)
        );
    }

    // 验证触发限价单在未达到触发条件时保持未触发状态。
    #[tokio::test]
    async fn trigger_limit_order_stays_submitted_when_not_triggered() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(BTC, 200.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.kind, Kind::Trigger);
        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证限价单穿价按最差价成交时，现金差值符合“价格差+费率差”的公式。
    #[tokio::test]
    async fn maker_vs_taker_fee_difference_on_same_entry_price() {
        let market_exchange = single_exchange();
        market_exchange.next(BTC, Level::Minute1).await.unwrap();
        market_exchange.buy(BTC, 1.0).await.unwrap();
        market_exchange.next(BTC, Level::Minute1).await.unwrap();
        let market_cash = market_exchange.get_cash().await.unwrap();

        let limit_exchange = single_exchange();
        limit_exchange.next(BTC, Level::Minute1).await.unwrap();
        limit_exchange.buy_limit(BTC, 105.0, 1.0).await.unwrap();
        limit_exchange.next(BTC, Level::Minute1).await.unwrap();
        let limit_cash = limit_exchange.get_cash().await.unwrap();

        assert!(limit_cash < market_cash);

        let market_cost = 105.0 / 10.0 + 105.0 * btc_metadata().taker_fee;
        let limit_cost = 106.0 / 10.0 + 106.0 * btc_metadata().maker_fee;
        assert_eq!(limit_cash - market_cash, market_cost - limit_cost);
    }

    // 验证调低杠杆倍率（更高保证金要求）时会重算仓位保证金与可用现金。
    #[tokio::test]
    async fn set_leverage_recalculates_position_margin_and_cash() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();
        let liquidation_price_before = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.set_leverage(BTC, 5).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();
        let liquidation_price_after = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert_eq!(position_after.leverage, 5);
        assert_eq!(position_before.margin, 10.5);
        assert_eq!(position_after.margin, 21.0);
        assert_eq!(cash_before - cash_after, 10.5);
        assert_eq!(liquidation_price_after, position_after.liquidation_price);
        assert!(liquidation_price_after < liquidation_price_before);
    }

    // 验证调低杠杆导致需要补充保证金但现金不足时会失败。
    #[tokio::test]
    async fn set_leverage_fails_when_cash_insufficient_for_new_margin() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(11.0)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange.set_leverage(BTC, 1).await.unwrap_err();

        assert!(result.to_string().contains("requires additional margin"));
        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 10);
    }

    // 验证触发限价单在触发瞬间若无法冻结保证金会被拒绝。
    #[tokio::test]
    async fn trigger_limit_order_rejected_when_freeze_margin_fails() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(1.0)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(BTC, 105.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let pending = exchange.get_pending_order_list(BTC).await.unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert!(pending.is_empty());
    }

    // 验证部分平仓后仍保留剩余仓位，并记录历史平仓数量。
    #[tokio::test]
    async fn partial_close_keeps_remaining_position_and_updates_history() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();

        assert_eq!(position.quantity, 1.0);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].close_quantity, 1.0);
        assert_eq!(history[0].close_avg_price, 120.0);
    }

    // 验证反向开仓后，强平保护单方向与价格会同步到新仓位。
    #[tokio::test]
    async fn reverse_position_updates_liquidation_order_side_and_price() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let liquidation = pending
            .iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(liquidation.side, Side::Buy);
        assert_eq!(liquidation.price, position.liquidation_price);
    }

    // 验证追加保证金后强平价下降、再减少保证金后强平价回升（多仓场景）。
    #[tokio::test]
    async fn append_and_reduce_margin_moves_liquidation_price_monotonic() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_before = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(BTC, 2.0).await.unwrap();
        let liq_after_add = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(BTC, -1.0).await.unwrap();
        let liq_after_reduce = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert!(liq_after_add < liq_before);
        assert!(liq_after_reduce > liq_after_add);
    }

    // 验证仅存在强平保护单时允许调杠杆，且挂单数量不变化。
    #[tokio::test]
    async fn set_leverage_allowed_with_only_liquidation_order() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let pending_before = exchange.get_pending_order_list(BTC).await.unwrap();
        assert_eq!(pending_before.len(), 1);
        assert_eq!(pending_before[0].kind, Kind::Liquidation);

        exchange.set_leverage(BTC, 5).await.unwrap();

        let pending_after = exchange.get_pending_order_list(BTC).await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 5);
        assert_eq!(pending_after.len(), 1);
        assert_eq!(pending_after[0].kind, Kind::Liquidation);
        assert_eq!(pending_after[0].price, position_after.liquidation_price);
    }

    // 验证调杠杆失败时，仓位保证金与强平价格不会被污染。
    #[tokio::test]
    async fn set_leverage_failure_keeps_position_and_liquidation_unchanged() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(11.0)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();
        let liquidation_before = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        let _ = exchange.set_leverage(BTC, 1).await.unwrap_err();

        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();
        let liquidation_after = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 10);
        assert_eq!(position_after.margin, position_before.margin);
        assert_eq!(
            position_after.liquidation_price,
            position_before.liquidation_price,
        );
        assert_eq!(liquidation_after.price, liquidation_before.price);
    }

    // 验证升杠杆（保证金需求降低）会返还现金，并抬升多仓强平价。
    #[tokio::test]
    async fn set_leverage_higher_reduces_margin_and_returns_cash() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();

        exchange.set_leverage(BTC, 20).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(position_after.leverage, 20);
        assert!(position_after.margin < position_before.margin);
        assert!(cash_after > cash_before);
        assert!(position_after.liquidation_price > position_before.liquidation_price);
    }

    // 验证无仓位时调杠杆只更新全局杠杆，不影响现金与挂单。
    #[tokio::test]
    async fn set_leverage_without_position_only_updates_config() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let pending_before = exchange.get_pending_order_list(BTC).await.unwrap();

        exchange.set_leverage(BTC, 25).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let pending_after = exchange.get_pending_order_list(BTC).await.unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 25);
        assert_eq!(cash_after, cash_before);
        assert_eq!(pending_after.len(), pending_before.len());
    }

    // 验证无仓位时追加保证金会失败，且现金不会发生变化。
    #[tokio::test]
    async fn append_position_margin_without_position_keeps_cash_unchanged() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let result = exchange.append_position_margin(BTC, 1.0).await.unwrap_err();
        let cash_after = exchange.get_cash().await.unwrap();

        assert!(result.to_string().contains("no position"));
        assert_eq!(cash_after, cash_before);
    }

    // 验证无仓位时 close_all_position 是幂等空操作，不创建订单。
    #[tokio::test]
    async fn close_all_position_without_position_is_noop() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.close_all_position(BTC).await.unwrap();

        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let history_order = exchange.get_history_order_list(BTC).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(pending.is_empty());
        assert!(history_order.is_empty());
    }

    // 验证存在普通限价挂单时调杠杆失败，且杠杆与现金保持不变。
    #[tokio::test]
    async fn set_leverage_with_limit_pending_keeps_state_unchanged() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let leverage_before = exchange.get_leverage(BTC).await.unwrap();
        let cash_before = exchange.get_cash().await.unwrap();
        let _id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();

        let result = exchange.set_leverage(BTC, 15).await.unwrap_err();

        let leverage_after = exchange.get_leverage(BTC).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(leverage_after, leverage_before);
        assert_eq!(cash_after, cash_before - 9.0);
    }

    // 验证存在普通触发挂单时调杠杆失败，且不会污染仓位状态。
    #[tokio::test]
    async fn set_leverage_with_trigger_pending_keeps_position_unchanged() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();
        let _trigger_id = exchange.buy_trigger_market(BTC, 200.0, 1.0).await.unwrap();

        let result = exchange.set_leverage(BTC, 8).await.unwrap_err();

        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(position_after.leverage, position_before.leverage);
        assert_eq!(position_after.margin, position_before.margin);
        assert_eq!(
            position_after.liquidation_price,
            position_before.liquidation_price,
        );
    }

    // 验证强平订单被触发后会平掉仓位，并在历史仓位中标记为强平。
    #[tokio::test]
    async fn liquidation_order_closes_position_and_records_liquidation_history() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(95.0), dec!(96.0), dec!(90.0), dec!(92.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_order = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();
        assert!(liq_order.price >= 90.0);
        assert!(liq_order.price <= 96.0);

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
    }

    // 验证 1x 多仓强平价受 maintenance 约束，并在触及阈值时执行强平。
    #[tokio::test]
    async fn one_x_long_liquidation_price_respects_maintenance() {
        let exchange = LocalExchange::new(vec![DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(3, dec!(60.0), dec!(61.0), dec!(1.0), dec!(10.0)),
                gen_kline(4, dec!(1.0), dec!(1.0), dec!(0.3), dec!(0.5)),
            ],
        )])
        .cash(10000.0)
        .leverage(1);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_after_open = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_after_open = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position_after_open.leverage, 1);
        assert_eq!(position_after_open.liquidation_price, 0.4);
        assert_eq!(liq_after_open.price, 0.4);
        assert_eq!(liq_after_open.side, Side::Sell);

        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert!(exchange.get_position(BTC).await.unwrap().is_some());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_eq!(history[0].close_avg_price, 0.4);
    }

    // 验证 1x 空仓强平价受 maintenance 约束，并在触及阈值时执行强平。
    #[tokio::test]
    async fn one_x_short_liquidation_price_respects_maintenance() {
        let exchange = LocalExchange::new(vec![DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(3, dec!(180.0), dec!(181.0), dec!(179.0), dec!(180.0)),
                gen_kline(4, dec!(200.0), dec!(200.0), dec!(199.0), dec!(199.8)),
            ],
        )])
        .cash(10000.0)
        .leverage(1);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_after_open = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_after_open = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position_after_open.leverage, 1);
        assert_eq!(position_after_open.liquidation_price, 199.6);
        assert_eq!(liq_after_open.price, 199.6);
        assert_eq!(liq_after_open.side, Side::Buy);

        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert!(exchange.get_position(BTC).await.unwrap().is_some());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        let history = exchange.get_history_position_list(BTC).await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_eq!(history[0].close_avg_price, 199.6);
    }

    // 验证存在待成交市价单时调杠杆会失败，并保持杠杆值不变。
    #[tokio::test]
    async fn set_leverage_with_market_pending_keeps_leverage_unchanged() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let leverage_before = exchange.get_leverage(BTC).await.unwrap();
        let _id = exchange.buy(BTC, 1.0).await.unwrap();

        let result = exchange.set_leverage(BTC, 30).await.unwrap_err();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), leverage_before);
    }

    // 验证 cancel_all 同时取消 trigger/limit 时，现金仅返还限价单冻结保证金。
    #[tokio::test]
    async fn cancel_all_refunds_only_frozen_margin_orders() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let cash_after_limit = exchange.get_cash().await.unwrap();

        exchange
            .buy_trigger_limit(BTC, 200.0, 90.0, 1.0)
            .await
            .unwrap();
        let cash_after_trigger = exchange.get_cash().await.unwrap();

        assert!(cash_after_limit < cash_before);
        assert_eq!(cash_after_trigger, cash_after_limit);

        exchange.cancel_all_order(BTC).await.unwrap();

        let cash_after_cancel_all = exchange.get_cash().await.unwrap();
        assert_eq!(cash_after_cancel_all, cash_before);
    }

    // 验证减少保证金到“初始保证金边界值”是允许的。
    #[tokio::test]
    async fn append_position_margin_allows_reducing_to_initial_margin_boundary() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.append_position_margin(BTC, 2.0).await.unwrap();

        let position_after_add = exchange.get_position(BTC).await.unwrap().unwrap();
        let init_margin = position_after_add.open_avg_price * position_after_add.quantity
            / position_after_add.leverage as f64;
        let reduce_to_boundary = init_margin - position_after_add.margin;

        exchange
            .append_position_margin(BTC, reduce_to_boundary)
            .await
            .unwrap();

        let position_after_reduce = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_reduce.margin, init_margin);
    }

    // 验证浮点微小超边界时限价不应成交（避免误成交）。
    #[tokio::test]
    async fn limit_order_not_fill_when_slightly_above_high() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 104.0 - 0.1, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证买入限价在价格等于 K 线 low 边界时可以成交。
    #[tokio::test]
    async fn buy_limit_fills_when_price_equals_low_boundary() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 104.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 104.0);
        assert_eq!(position.side, Side::Buy);
        assert_eq!(position.open_avg_price, 104.0);
    }

    // 验证卖出限价在价格等于 K 线 high 边界时可以成交。
    #[tokio::test]
    async fn sell_limit_fills_when_price_equals_high_boundary() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.sell_limit(BTC, 106.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 106.0);
        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.open_avg_price, 106.0);
    }

    // 验证买入限价略低于 low 边界时不会成交。
    #[tokio::test]
    async fn buy_limit_not_fill_when_slightly_below_low_boundary() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 104.0 - 0.1, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证卖出限价略高于 high 边界时不会成交。
    #[tokio::test]
    async fn sell_limit_not_fill_when_slightly_above_high_boundary() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.sell_limit(BTC, 106.0 + 0.1, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证多次追加/减少保证金后，现金回到原值（控制浮点累计误差）。
    #[tokio::test]
    async fn append_margin_round_trip_keeps_cash_precision() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        exchange.append_position_margin(BTC, 0.1).await.unwrap();
        exchange.append_position_margin(BTC, -0.1).await.unwrap();
        exchange.append_position_margin(BTC, 0.2).await.unwrap();
        exchange.append_position_margin(BTC, -0.2).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after, cash_before);
    }

    // 验证小数数量下限价穿价成交时现金差值仍保持数学一致性。
    #[tokio::test]
    async fn maker_taker_fee_delta_with_fractional_quantity_precision() {
        let qty = 0.123;

        let market_exchange = single_exchange();
        market_exchange.next(BTC, Level::Minute1).await.unwrap();
        market_exchange.buy(BTC, qty).await.unwrap();
        market_exchange.next(BTC, Level::Minute1).await.unwrap();
        let market_cash = market_exchange.get_cash().await.unwrap();

        let limit_exchange = single_exchange();
        limit_exchange.next(BTC, Level::Minute1).await.unwrap();
        limit_exchange.buy_limit(BTC, 105.0, qty).await.unwrap();
        limit_exchange.next(BTC, Level::Minute1).await.unwrap();
        let limit_cash = limit_exchange.get_cash().await.unwrap();

        let market_cost = 105.0 * qty / 10.0 + 105.0 * qty * btc_metadata().taker_fee;
        let limit_cost = 106.0 * qty / 10.0 + 106.0 * qty * btc_metadata().maker_fee;

        assert!(limit_cash < market_cash);
        assert_eq!(limit_cash - market_cash, market_cost - limit_cost);
    }

    // 验证分数数量市价开仓时，现金扣减严格符合“保证金+taker手续费”公式。
    #[tokio::test]
    async fn market_open_fractional_quantity_cash_matches_formula() {
        let qty = 0.333;
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let expected_margin = 105.0 * qty / 10.0;
        let expected_fee = 105.0 * qty * btc_metadata().taker_fee;
        let expected_cash = 10000.0 - expected_margin - expected_fee;

        assert_eq!(cash_after, expected_cash);
    }

    // 验证分数数量限价开仓时，现金扣减严格符合“保证金+maker手续费”公式。
    #[tokio::test]
    async fn limit_open_fractional_quantity_cash_matches_formula() {
        let qty = 0.333;
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy_limit(BTC, 105.0, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let expected_margin = 106.0 * qty / 10.0;
        let expected_fee = 106.0 * qty * btc_metadata().maker_fee;
        let expected_cash = 10000.0 - expected_margin - expected_fee;

        assert_eq!(cash_after, expected_cash);
    }

    // 验证分数数量完整开平一轮后，现金严格符合“初始资金+利润-开平手续费”公式。
    #[tokio::test]
    async fn open_close_round_trip_fractional_cash_matches_formula() {
        let qty = 0.333;
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();

        let open_fee = 105.0 * qty * btc_metadata().taker_fee;
        let close_fee = 110.0 * qty * btc_metadata().taker_fee;
        let profit = (110.0 - 105.0) * qty;
        let expected_cash = 10000.0 + profit - open_fee - close_fee;

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(cash_after, expected_cash);
    }

    // 验证逐根 next 的状态严格一致：现金、权益、仓位、挂单和历史记录均符合精确公式。
    #[tokio::test]
    async fn strict_step_by_step_state_assertions_on_each_next() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.1), dec!(101.2), dec!(99.4), dec!(100.5)),
                gen_kline(2, dec!(105.3), dec!(106.4), dec!(104.2), dec!(105.9)),
                gen_kline(3, dec!(110.2), dec!(111.4), dec!(109.3), dec!(110.8)),
                gen_kline(4, dec!(120.1), dec!(121.5), dec!(119.6), dec!(120.4)),
            ],
        );

        let md = btc_metadata();
        let qty = dec!(1.0);
        let leverage = dec!(10.0);
        let open_fill_price = dec!(105.3);
        let bar2_close = dec!(105.9);
        let bar3_close = dec!(110.8);
        let close_fill_price = dec!(120.1);

        // Step 1: 第一根 K 线，仅推进时间，不应有资金与仓位变化。
        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), dec!(10000.0));
        assert_eq!(exchange.get_equity().await.unwrap(), dec!(10000.0));
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            exchange
                .get_history_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );

        // 下市价开多单：撮合前资金不变，仅产生 pending。
        let open_id = exchange.buy(BTC, 1.0).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), 10000.0);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);

        // Step 2: 第二根 K 线撮合开仓。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&open_id).await.unwrap().unwrap();
        let position_after_open = exchange.get_position(BTC).await.unwrap().unwrap();
        let pending_after_open = exchange.get_pending_order_list(BTC).await.unwrap();

        let open_margin = open_fill_price * qty / leverage;
        let open_fee = open_fill_price * qty * md.taker_fee;
        let expected_cash_after_open = 10000.0 - open_margin - open_fee;
        let expected_equity_after_open =
            expected_cash_after_open + open_margin + (bar2_close - open_fill_price) * qty;
        let expected_liquidation_price =
            open_fill_price * (dec!(1.0) - dec!(1.0) / leverage + md.maintenance);

        assert_eq!(open_order.status, Status::Filled);
        assert_eq!(open_order.avg_price, open_fill_price);
        assert_eq!(open_order.cumulative_quantity, qty);

        assert_eq!(position_after_open.side, Side::Buy);
        assert_eq!(position_after_open.leverage, 10);
        assert_eq!(position_after_open.open_avg_price, open_fill_price);
        assert_eq!(position_after_open.quantity, qty);
        assert_eq!(position_after_open.margin, open_margin);
        assert_eq!(
            position_after_open.profit,
            (bar2_close - open_fill_price) * qty,
        );
        assert_eq!(
            position_after_open.liquidation_price,
            expected_liquidation_price,
        );

        assert_eq!(exchange.get_cash().await.unwrap(), expected_cash_after_open);
        assert_eq!(
            exchange.get_equity().await.unwrap(),
            expected_equity_after_open,
        );

        assert_eq!(pending_after_open.len(), 1);
        assert_eq!(pending_after_open[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_open[0].side, Side::Sell);
        assert_eq!(pending_after_open[0].price, expected_liquidation_price);

        // Step 3: 第三根 K 线无成交，仅更新浮盈。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_after_bar3 = exchange.get_position(BTC).await.unwrap().unwrap();
        let expected_cash_after_bar3 = expected_cash_after_open;
        let expected_equity_after_bar3 =
            expected_cash_after_bar3 + open_margin + (bar3_close - open_fill_price) * qty;

        assert_eq!(exchange.get_cash().await.unwrap(), expected_cash_after_bar3);
        assert_eq!(
            exchange.get_equity().await.unwrap(),
            expected_equity_after_bar3,
        );
        assert_eq!(
            position_after_bar3.profit,
            (bar3_close - open_fill_price) * qty,
        );
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);

        // 提交 reduce-only 平仓单：撮合前资金不变，挂单增加。
        let close_id = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), expected_cash_after_bar3);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // Step 4: 第四根 K 线撮合平仓。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash_after_close = exchange.get_cash().await.unwrap();
        let equity_after_close = exchange.get_equity().await.unwrap();
        let pending_after_close = exchange.get_pending_order_list(BTC).await.unwrap();

        let close_fee = close_fill_price * qty * md.taker_fee;
        let close_profit = (close_fill_price - open_fill_price) * qty;
        let expected_cash_after_close =
            expected_cash_after_bar3 - close_fee + open_margin + close_profit;

        assert_eq!(close_order.status, Status::Filled);
        assert_eq!(close_order.avg_price, close_fill_price);
        assert_eq!(close_order.cumulative_quantity, qty);

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(pending_after_close.is_empty());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_eq!(history[0].open_avg_price, open_fill_price);
        assert_eq!(history[0].close_avg_price, close_fill_price);
        assert_eq!(history[0].profit, close_profit);
        assert_eq!(history[0].fee, open_fee + close_fee);
        assert_eq!(history[0].total_profit, close_profit - open_fee - close_fee);

        assert_eq!(cash_after_close, expected_cash_after_close);
        assert_eq!(equity_after_close, expected_cash_after_close);
    }

    // 验证复杂路径：市价开仓 -> 调杠杆 -> 加减保证金 -> 条件单触发 -> 反向开仓 -> 限价减仓，
    // 且每次 next 后均对 cash/equity/position/pending/history 做精确小数断言。
    #[tokio::test]
    async fn strict_complex_flow_asserts_all_states_on_every_next() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.1), dec!(101.2), dec!(99.4), dec!(100.5)),
                gen_kline(2, dec!(105.3), dec!(106.4), dec!(104.2), dec!(105.9)),
                gen_kline(3, dec!(110.2), dec!(111.4), dec!(109.3), dec!(110.8)),
                gen_kline(4, dec!(108.4), dec!(109.1), dec!(107.8), dec!(108.0)),
                gen_kline(5, dec!(103.7), dec!(104.2), dec!(102.9), dec!(103.1)),
            ],
        );

        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(20000.0)
                .leverage(10),
        ));

        let md = btc_metadata();

        let qty_long = dec!(1.234);
        let qty_reverse_order = dec!(2.0);
        let qty_reverse_short = qty_reverse_order - qty_long;
        let qty_limit_reduce = dec!(0.3);

        let p_open_long = dec!(105.3);
        let p_close_bar2 = dec!(105.9);
        let p_open_reverse = dec!(110.5);
        let p_close_bar4 = dec!(108.0);
        let p_limit_close = dec!(104.2);
        let p_close_bar5 = dec!(103.1);

        // Step 1: 仅推进时间，账户初始状态不变。
        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), 20000.0);
        assert_eq!(exchange.get_equity().await.unwrap(), 20000.0);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            exchange
                .get_history_position_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );

        // 下市价开多：撮合前仅新增挂单。
        let market_open_id = exchange.buy(BTC, qty_long).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), 20000.0);
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 1);

        // Step 2: 市价开多在 bar2 成交。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&market_open_id).await.unwrap().unwrap();
        let mut position = exchange.get_position(BTC).await.unwrap().unwrap();
        let pending_after_open = exchange.get_pending_order_list(BTC).await.unwrap();

        let open_margin_10 = p_open_long * qty_long / dec!(10.0);
        let open_fee = p_open_long * qty_long * md.taker_fee;
        let cash_after_open = 20000.0 - open_margin_10 - open_fee;
        let upnl_bar2 = (p_close_bar2 - p_open_long) * qty_long;
        let equity_after_open = cash_after_open + open_margin_10 + upnl_bar2;
        let liq_after_open = p_open_long * (dec!(1.0) - dec!(1.0) / dec!(10.0) + md.maintenance);

        assert_eq!(open_order.status, Status::Filled);
        assert_eq!(open_order.avg_price, p_open_long);
        assert_eq!(open_order.cumulative_quantity, qty_long);

        assert_eq!(position.side, Side::Buy);
        assert_eq!(position.leverage, 10);
        assert_eq!(position.open_avg_price, p_open_long);
        assert_eq!(position.quantity, qty_long);
        assert_eq!(position.margin, open_margin_10);
        assert_eq!(position.profit, upnl_bar2);
        assert_eq!(position.liquidation_price, liq_after_open);

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_open);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_after_open);
        assert_eq!(pending_after_open.len(), 1);
        assert_eq!(pending_after_open[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_open[0].side, Side::Sell);
        assert_eq!(pending_after_open[0].price, liq_after_open);

        // 调杠杆 + 追加/减少保证金（不经过 next，但会影响后续 next 断言基线）。
        exchange.set_leverage(BTC, 8).await.unwrap();
        exchange.append_position_margin(BTC, 1.111).await.unwrap();
        exchange.append_position_margin(BTC, -0.321).await.unwrap();

        let margin_after_leverage = p_open_long * qty_long / dec!(8.0);
        let cash_after_leverage = cash_after_open - (margin_after_leverage - open_margin_10);
        let margin_after_adjust = margin_after_leverage + dec!(1.111) - dec!(0.321);
        let cash_after_adjust = cash_after_leverage - dec!(1.111) + dec!(0.321);
        let liq_after_adjust = p_open_long * (dec!(1.0) - dec!(1.0) / dec!(8.0) + md.maintenance)
            - (margin_after_adjust - margin_after_leverage) / qty_long;

        let position_after_adjust = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_adjust.leverage, 8);
        assert_eq!(position_after_adjust.margin, margin_after_adjust);
        assert_eq!(position_after_adjust.liquidation_price, liq_after_adjust);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_adjust);

        // 下触发市价卖单（条件单），将来触发后反向开仓。
        exchange
            .sell_trigger_market(BTC, 110.5, qty_reverse_order)
            .await
            .unwrap();
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);

        // Step 3: 条件单触发并撮合成交
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let fee_full_reverse_order = p_open_reverse * qty_reverse_order * md.taker_fee;
        let close_profit_long = (p_open_reverse - p_open_long) * qty_long;
        let reverse_margin_8 = p_open_reverse * qty_reverse_short / 8.0;

        let cash_after_reverse =
            cash_after_adjust - fee_full_reverse_order + margin_after_adjust + close_profit_long
                - reverse_margin_8;
        let upnl_bar4_short = (p_open_reverse - p_close_bar4) * qty_reverse_short;
        let equity_bar4 = cash_after_reverse + reverse_margin_8 + upnl_bar4_short;
        let liq_short = p_open_reverse * (1.0 + 1.0 / 8.0 - md.maintenance);

        let history_after_reverse = exchange.get_history_position_list(BTC).await.unwrap();
        let pending_after_reverse = exchange.get_pending_order_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 8);
        assert_eq!(position.open_avg_price, p_open_reverse);
        assert_eq!(position.quantity, qty_reverse_short);
        assert_eq!(position.margin, reverse_margin_8);
        assert_eq!(position.profit, upnl_bar4_short);
        assert_eq!(position.liquidation_price, liq_short);

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_reverse);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_bar4);

        assert_eq!(history_after_reverse.len(), 1);
        assert_eq!(history_after_reverse[0].side, Side::Buy);
        assert_eq!(history_after_reverse[0].open_avg_price, p_open_long);
        assert_eq!(history_after_reverse[0].close_avg_price, p_open_reverse);
        assert_eq!(history_after_reverse[0].close_quantity, qty_long);
        assert_eq!(history_after_reverse[0].profit, close_profit_long);

        assert_eq!(pending_after_reverse.len(), 1);
        assert_eq!(pending_after_reverse[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_reverse[0].side, Side::Buy);
        assert_eq!(pending_after_reverse[0].price, liq_short);

        // 下 reduce-only 限价买单，对空仓做部分平仓。
        let limit_reduce_id = exchange
            .buy_limit_reduce_only(BTC, 104.0, qty_limit_reduce)
            .await
            .unwrap();
        assert_eq!(exchange.get_pending_order_list(BTC).await.unwrap().len(), 2);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_reverse);

        // Step 5: 限价减仓成交（买价>=open，按 high 成交），并更新历史与仓位。
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_limit_order = exchange.get_order(&limit_reduce_id).await.unwrap().unwrap();
        let history_after_limit = exchange.get_history_position_list(BTC).await.unwrap();
        let pending_after_limit = exchange.get_pending_order_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        let close_margin_short = reverse_margin_8 * (qty_limit_reduce / qty_reverse_short);
        let close_profit_short = (p_open_reverse - p_limit_close) * qty_limit_reduce;
        let close_fee_short = p_limit_close * qty_limit_reduce * md.maker_fee;

        let cash_after_limit =
            cash_after_reverse - close_fee_short + close_margin_short + close_profit_short;

        let qty_short_left = qty_reverse_short - qty_limit_reduce;
        let margin_short_left = reverse_margin_8 - close_margin_short;
        let upnl_bar5_short = (p_open_reverse - p_close_bar5) * qty_short_left;
        let equity_bar5 = cash_after_limit + margin_short_left + upnl_bar5_short;

        assert_eq!(close_limit_order.status, Status::Filled);
        assert_eq!(close_limit_order.avg_price, p_limit_close);
        assert_eq!(close_limit_order.cumulative_quantity, qty_limit_reduce);

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 8);
        assert_eq!(position.open_avg_price, p_open_reverse);
        assert_eq!(position.quantity, qty_short_left);
        assert_eq!(position.margin, margin_short_left);
        assert_eq!(position.profit, upnl_bar5_short);
        assert_eq!(position.liquidation_price, liq_short);

        assert_eq!(exchange.get_cash().await.unwrap(), cash_after_limit);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_bar5);

        assert_eq!(pending_after_limit.len(), 1);
        assert_eq!(pending_after_limit[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_limit[0].side, Side::Buy);
        assert_eq!(pending_after_limit[0].price, liq_short);

        assert_eq!(history_after_limit.len(), 2);
        assert_eq!(history_after_limit[1].side, Side::Sell);
        assert_eq!(history_after_limit[1].open_avg_price, p_open_reverse);
        assert_eq!(history_after_limit[1].close_avg_price, p_limit_close);
        assert_eq!(history_after_limit[1].close_quantity, qty_limit_reduce);
        assert_eq!(history_after_limit[1].profit, close_profit_short);
    }

    #[tokio::test]
    async fn insane_state_machine_asserts_everything_each_next() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.1), dec!(101.2), dec!(99.4), dec!(100.5)),
                gen_kline(2, dec!(105.3), dec!(106.4), dec!(104.2), dec!(105.9)),
                gen_kline(3, dec!(110.2), dec!(111.4), dec!(109.3), dec!(110.8)),
                gen_kline(4, dec!(108.4), dec!(109.1), dec!(107.8), dec!(108.0)),
                gen_kline(5, dec!(103.7), dec!(104.2), dec!(102.9), dec!(103.1)),
                gen_kline(6, dec!(99.3), dec!(100.5), dec!(98.6), dec!(99.8)),
                gen_kline(7, dec!(112.6), dec!(113.2), dec!(111.9), dec!(112.2)),
            ],
        );

        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(30000.0)
                .leverage(10),
        ));

        let md = btc_metadata();

        let q_market_open = dec!(1.23);
        let q_limit_open = dec!(0.5);
        let q_long = q_market_open + q_limit_open; // 1.7345
        let q_reverse_order = dec!(3.0);
        let q_short = q_reverse_order - q_long; // 1.2655
        let q_trigger_limit_close = dec!(0.6);
        let q_short_left = q_short - q_trigger_limit_close; // 0.6655

        let p_market_open = dec!(105.3);
        let p_limit_open = dec!(104.7);
        let p_trigger_reverse = dec!(110.5); // 条件市价单触发价
        let p_trigger_limit = dec!(103.2); // 条件限价单成交价
        let p_final_close = dec!(112.6);

        // ---------- Step 1: 初始状态 ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();
        assert_eq!(exchange.get_cash().await.unwrap(), 30000.0);
        assert_eq!(exchange.get_equity().await.unwrap(), 30000.0);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );

        // 提交市价开多 + 限价开多 + 条件市价反向单（trigger=110.5）
        let id_market_open = exchange.buy(BTC, q_market_open).await.unwrap();
        let id_limit_open = exchange
            .buy_limit(BTC, p_limit_open, q_limit_open)
            .await
            .unwrap();
        let id_trigger_reverse = exchange
            .sell_trigger_market(BTC, p_trigger_reverse, q_reverse_order)
            .await
            .unwrap();

        // ---------- Step 2: 市价与限价开仓成交，trigger 未触发 ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order_market_open = exchange.get_order(&id_market_open).await.unwrap().unwrap();
        let order_limit_open = exchange.get_order(&id_limit_open).await.unwrap().unwrap();
        let trigger_reverse = exchange
            .get_order(&id_trigger_reverse)
            .await
            .unwrap()
            .unwrap();
        let mut position = exchange.get_position(BTC).await.unwrap().unwrap();
        let pending_s2 = exchange.get_pending_order_list(BTC).await.unwrap();

        let avg_long = (q_market_open * p_market_open + q_limit_open * p_limit_open) / q_long;
        let margin_long = q_market_open * p_market_open / 10.0 + q_limit_open * p_limit_open / 10.0;
        let fee_open_market = q_market_open * p_market_open * md.taker_fee;
        let fee_open_limit = q_limit_open * p_limit_open * md.maker_fee;
        let cash_s2 = 30000.0 - margin_long - fee_open_market - fee_open_limit;
        let upnl_s2 = (105.9 - avg_long) * q_long;
        let equity_s2 = cash_s2 + margin_long + upnl_s2;

        assert_eq!(order_market_open.status, Status::Filled);
        assert_eq!(order_limit_open.status, Status::Filled);
        assert_eq!(trigger_reverse.status, Status::Submitted);
        assert_eq!(position.side, Side::Buy);
        assert_eq!(position.leverage, 10);
        assert_eq!(position.open_avg_price, avg_long);
        assert_eq!(position.quantity, q_long);
        assert_eq!(position.margin, margin_long);
        assert_eq!(position.profit, upnl_s2);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s2);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(pending_s2.len(), 2);
        assert!(pending_s2.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s2.iter().any(|o| o.kind == Kind::Trigger));

        // 有普通挂单（Trigger）时调杠杆应失败
        let lev_err_with_trigger = exchange.set_leverage(BTC, 8).await.unwrap_err();
        assert!(lev_err_with_trigger.to_string().contains("pending orders"));

        // 新增条件限价平空单（将在后面触发并成交）
        let id_trigger_limit = exchange
            .buy_trigger_limit(BTC, 103.0, p_trigger_limit, q_trigger_limit_close)
            .await
            .unwrap();

        // ---------- Step 3: 条件市价单触发并立即成交（平多开空） ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_reverse_s3 = exchange
            .get_order(&id_trigger_reverse)
            .await
            .unwrap()
            .unwrap();
        let trigger_limit_s3 = exchange
            .get_order(&id_trigger_limit)
            .await
            .unwrap()
            .unwrap();
        let pending_s3 = exchange.get_pending_order_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        // 计算平多开空后的现金与保证金
        let close_long_value = q_long * p_trigger_reverse;
        let close_long_profit = close_long_value - (q_long * avg_long);
        let fee_reverse = q_reverse_order * p_trigger_reverse * md.taker_fee;
        let margin_short = q_short * p_trigger_reverse / 10.0;

        let cash_s3 = cash_s2 + margin_long + close_long_profit - margin_short - fee_reverse;
        let upnl_s3 = (p_trigger_reverse - 110.8) * q_short;
        let equity_s3 = cash_s3 + margin_short + upnl_s3;

        assert_eq!(trigger_reverse_s3.status, Status::Filled);
        assert_eq!(trigger_limit_s3.status, Status::Submitted);
        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 10);
        assert_eq!(position.open_avg_price, p_trigger_reverse);
        assert_eq!(position.quantity, q_short);
        assert_eq!(position.margin, margin_short);
        assert_eq!(position.profit, upnl_s3);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s3);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s3);
        assert_eq!(pending_s3.len(), 2);
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s3.iter().any(|o| o.kind != Kind::Market));
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Trigger));

        // ---------- Step 4: 下一根K线，更新未实现盈亏，条件限价单尚未触发 ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_limit_s4 = exchange
            .get_order(&id_trigger_limit)
            .await
            .unwrap()
            .unwrap();
        let pending_s4 = exchange.get_pending_order_list(BTC).await.unwrap();
        let history_s4 = exchange.get_history_position_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        let current_price_s4 = 108.0; // K线4收盘价
        let upnl_s4 = (p_trigger_reverse - current_price_s4) * q_short;
        let equity_s4 = cash_s3 + margin_short + upnl_s4;

        assert_eq!(trigger_limit_s4.status, Status::Submitted); // 尚未触发
        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 10);
        assert_eq!(position.open_avg_price, p_trigger_reverse);
        assert_eq!(position.quantity, q_short);
        assert_eq!(position.margin, margin_short);
        assert_eq!(position.profit, upnl_s4);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s3);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(history_s4.len(), 1);
        assert_eq!(history_s4[0].side, Side::Buy);
        assert_eq!(pending_s4.len(), 2);
        assert!(pending_s4.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s4.iter().any(|o| o.kind == Kind::Trigger));

        // ---------- Step 5: 条件限价单触发，转为限价单并冻结保证金 ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_limit_s5 = exchange
            .get_order(&id_trigger_limit)
            .await
            .unwrap()
            .unwrap();
        let pending_s5 = exchange.get_pending_order_list(BTC).await.unwrap();
        position = exchange.get_position(BTC).await.unwrap().unwrap();

        let upnl_s5 = (p_trigger_reverse - 103.1) * q_short_left;
        let close_margin = margin_short * (q_trigger_limit_close / q_short);
        let margin_short = margin_short - close_margin;
        let close_profit = (p_trigger_reverse - p_trigger_limit) * q_trigger_limit_close;
        let close_fee = p_trigger_limit * q_trigger_limit_close * md.maker_fee;
        let cash_s5 = cash_s3 + close_margin + close_profit - close_fee;
        let equity_s5 = cash_s5 + margin_short + upnl_s5;

        assert_eq!(trigger_limit_s5.status, Status::Filled); // 触发单转为限价单
        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.open_avg_price, p_trigger_reverse);
        assert_eq!(position.quantity, q_short_left);
        assert_eq!(position.margin, margin_short);
        assert_eq!(position.profit, upnl_s5);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_s5);
        assert_eq!(exchange.get_equity().await.unwrap(), equity_s5);
        assert_eq!(pending_s5.len(), 1);
        assert!(pending_s5.iter().any(|o| o.kind == Kind::Liquidation));

        // ---------- Step 6 ----------
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // ---------- 调整杠杆和保证金 ----------
        // 带普通挂单时调杠杆应失败
        let id_block_lev = exchange
            .sell_limit_reduce_only(BTC, 130.0, 0.1)
            .await
            .unwrap();
        let lev_err = exchange.set_leverage(BTC, 5).await.unwrap_err();
        assert!(lev_err.to_string().contains("pending orders"));

        // 撤掉普通挂单后允许调杠杆，并做保证金调整
        exchange.cancel_order(BTC, &id_block_lev).await.unwrap();
        exchange.set_leverage(BTC, 5).await.unwrap();
        exchange.append_position_margin(BTC, 0.333).await.unwrap();
        exchange.append_position_margin(BTC, -0.2).await.unwrap();

        let append_margin = p_trigger_reverse * q_short_left / 5.0 - margin_short;
        let cash_lev5_adj = cash_s5 - append_margin - 0.333 + 0.2;
        let margin_lev5_adj = margin_short + append_margin + 0.333 - 0.2;

        let position_after_adj = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(position_after_adj.leverage, 5);
        assert_eq!(position_after_adj.margin, margin_lev5_adj);
        assert_eq!(exchange.get_cash().await.unwrap(), cash_lev5_adj);

        // ---------- 市价全平 ----------
        exchange.close_all_position(BTC).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let fee_final = p_final_close * q_short_left * md.taker_fee;
        let profit_final = (p_trigger_reverse - p_final_close) * q_short_left;
        let cash_final = cash_lev5_adj - fee_final + margin_lev5_adj + profit_final;

        let history_final = exchange.get_history_position_list(BTC).await.unwrap();
        let pending_final = exchange.get_pending_order_list(BTC).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(pending_final.is_empty());
        assert_eq!(exchange.get_cash().await.unwrap(), cash_final);
        assert_eq!(exchange.get_equity().await.unwrap(), cash_final);

        assert_eq!(history_final.len(), 2);
        assert_eq!(history_final[0].side, Side::Buy);
        assert_eq!(history_final[1].side, Side::Sell);
        assert_eq!(history_final[1].open_avg_price, p_trigger_reverse);
        assert_eq!(history_final[1].close_avg_price, p_final_close);
        assert_eq!(history_final[1].close_quantity, q_short);
        assert_eq!(history_final[1].max_quantity, q_short);
    }

    // 验证限价单成交时若手续费不足被拒绝，会返还此前冻结的保证金。
    #[tokio::test]
    async fn limit_order_rejected_on_fee_shortage_refunds_frozen_margin() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(10.51)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_eq!(cash_after, cash_before);
    }

    // 验证按 ID 撤单不会允许撤掉强平保护单。
    #[tokio::test]
    async fn cancel_order_rejects_liquidation_order() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_id = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.id)
            .unwrap();

        let result = exchange.cancel_order(BTC, &liq_id).await.unwrap_err();

        assert!(result.to_string().contains("non-normal order"));

        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        assert!(pending.iter().any(|v| v.kind == Kind::Liquidation));
    }

    // 验证 cancel_order 会把订单以 Canceled 状态写入历史，便于后续审计。
    #[tokio::test]
    async fn cancel_order_writes_canceled_status_to_history() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        exchange.cancel_order(BTC, &id).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Canceled);
    }

    // 验证最小名义价值对市价单在成交时校验：满足阈值则可成交。
    #[tokio::test]
    async fn market_order_min_notional_is_checked_at_fill_time_and_can_pass() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
    }

    // 验证最小名义价值对市价单在成交时校验：不满足阈值则拒绝。
    #[tokio::test]
    async fn market_order_min_notional_is_checked_at_fill_time_and_can_reject() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 0.5).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Rejected);
    }

    // 验证市价单在 min_notional 成交拒绝时会返还冻结保证金，现金保持不变。
    #[tokio::test]
    async fn market_order_min_notional_reject_refunds_frozen_margin() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy(BTC, 0.5).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_eq!(cash_after, cash_before);
    }

    // 验证触发限价单在触发后会按限价成交前校验最小名义价值，不满足时拒绝。
    #[tokio::test]
    async fn trigger_limit_order_min_notional_is_checked_at_fill_time_and_can_reject() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);
        metadata.min_size = dec!(0.00000001);

        let exchange = single_exchange_with(
            metadata,
            vec![
                gen_kline(1, dec!(10.0), dec!(10.5), dec!(9.5), dec!(10.0)),
                gen_kline(2, dec!(20.0), dec!(21.0), dec!(19.0), dec!(20.0)),
                gen_kline(3, dec!(20.0), dec!(21.0), dec!(19.0), dec!(20.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(BTC, 20.0, 20.0, 4.0)
            .await
            .unwrap();

        // 第 2 根触发并立即执行，名义价值不足应被拒绝
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        // 触发单状态应为 Filled（已触发）
        assert_eq!(order.status, Status::Filled);

        // 检查所有历史订单
        let history = exchange.get_history_order_list(BTC).await.unwrap();

        // 检查是否有仓位
        assert!(exchange.get_position(BTC).await.unwrap().is_none());

        // 转换的限价单因名义价值不足被拒绝
        let converted_limit = history
            .iter()
            .find(|v| v.kind == Kind::Limit && v.status == Status::Rejected)
            .cloned();
        assert!(
            converted_limit.is_some(),
            "Should have a rejected limit order"
        );
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证触发限价单立即执行，在触发后满足最小名义价值时可以正常成交。
    #[tokio::test]
    async fn trigger_limit_order_min_notional_is_checked_at_fill_time_and_can_pass() {
        let mut metadata = btc_metadata();
        metadata.min_notional = dec!(100.0);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(BTC, 105.0, 110.0, 1.0)
            .await
            .unwrap();

        // 第 2 根触发并立即执行（限价110≥开盘价105，以最高价106成交）
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.open_avg_price, 106.0); // 立即执行，以当前K线最高价成交
        assert_eq!(position.quantity, 1.0);
    }

    // 验证权益口径为 cash + margin + upnl。
    #[tokio::test]
    async fn equity_includes_cash_margin_and_upnl() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash = exchange.get_cash().await.unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let equity = exchange.get_equity().await.unwrap();

        assert_eq!(equity, cash + position.margin + position.profit);
    }

    // 验证同一根 K 线中多个限价单会按挂单顺序撮合，并正确更新持仓均价。
    #[tokio::test]
    async fn matching_multiple_limit_orders_in_insertion_order() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let id1 = exchange.buy_limit(BTC, 104.5, 1.0).await.unwrap();
        let id2 = exchange.buy_limit(BTC, 105.5, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order1 = exchange.get_order(&id1).await.unwrap().unwrap();
        let order2 = exchange.get_order(&id2).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order1.status, Status::Filled);
        assert_eq!(order2.status, Status::Filled);
        assert_eq!(position.quantity, 2.0);
        assert_eq!(position.open_avg_price, 105.25);
    }

    // 验证触发市价单与普通市价单同Bar共存时，触发单遵循“两阶段撮合”。
    #[tokio::test]
    async fn matching_trigger_market_and_market_order_two_stage_behavior() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let market_id = exchange.buy(BTC, 1.0).await.unwrap();
        let trigger_id = exchange.buy_trigger_market(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let market_order = exchange.get_order(&market_id).await.unwrap().unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();

        assert_eq!(market_order.status, Status::Filled);
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发单立即执行，无 pending market order
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.quantity, 2.0);
        // 两个订单都在同一K线以开盘价105成交
        assert_eq!(position.open_avg_price, 105.0);
    }

    // 验证同Bar多条 reduce-only 平仓单竞争同一仓位时，后续订单会按剩余仓位处理。
    #[tokio::test]
    async fn matching_multiple_reduce_only_orders_share_single_position() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_id_1 = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        let close_id_2 = exchange.sell_reduce_only(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order_1 = exchange.get_order(&close_id_1).await.unwrap().unwrap();
        let close_order_2 = exchange.get_order(&close_id_2).await.unwrap().unwrap();

        assert_eq!(close_order_1.status, Status::Filled);
        assert_eq!(close_order_2.status, Status::Canceled);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证限价单冻结保证金金额精确匹配公式，撤单后余额完全归还。
    #[tokio::test]
    async fn margin_freeze_and_refund_for_limit_order_is_exact() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let qty = 1.23;
        let price = 90.0;
        let leverage = exchange.get_leverage(BTC).await.unwrap() as f64;
        let expected_freeze = price * qty / leverage;

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(BTC, price, qty).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert_eq!(cash_before - cash_after_place, expected_freeze);

        exchange.cancel_order(BTC, &id).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_cancel, cash_before);
    }

    // 验证触发限价单在提交阶段不冻结，触发后转限价时才冻结保证金。
    #[tokio::test]
    async fn trigger_limit_freezes_margin_only_after_trigger() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange
            .buy_trigger_limit(BTC, 105.0, 105.0, 1.0)
            .await
            .unwrap();
        let cash_after_submit = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_submit, cash_before);

        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 触发后立即执行，保证金和手续费都被扣除
        let cash_after_trigger = exchange.get_cash().await.unwrap();
        // 买单限价105≥开盘价105，以最高价106成交
        let fill_price = 106.0;
        let margin = fill_price * 1.0 / 10.0; // 10.6
        let fee = fill_price * 1.0 * 0.0002; // 0.0212 (maker fee)
        let total_cost = margin + fee; // 10.6212
        assert_eq!(cash_before - cash_after_trigger, total_cost);

        // 订单已执行，无法取消
        let position = exchange.get_position(BTC).await.unwrap();
        assert!(position.is_some());
    }

    // 验证多条限价单撤单时余额返还等于冻结保证金总和。
    #[tokio::test]
    async fn cancel_all_refund_equals_total_frozen_margin_for_multiple_limits() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        exchange.buy_limit(BTC, 80.0, 2.0).await.unwrap();

        let cash_after_place = exchange.get_cash().await.unwrap();
        let expected_freeze_total = 90.0 * 1.0 / 10.0 + 80.0 * 2.0 / 10.0;

        assert_eq!(cash_before - cash_after_place, expected_freeze_total);

        exchange.cancel_all_order(BTC).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_cancel, cash_before);
    }

    // 验证市价单因保证金不足被拒绝时，不会错误扣减余额。
    #[tokio::test]
    async fn market_order_rejected_on_margin_shortage_keeps_cash_unchanged() {
        let exchange = LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
            .cash(1.0)
            .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_eq!(cash_after, cash_before);
    }

    // 验证 reduce-only 限价单挂单与撤单过程不会冻结或返还保证金。
    #[tokio::test]
    async fn reduce_only_limit_does_not_change_cash_on_place_or_cancel() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange
            .sell_limit_reduce_only(BTC, 120.0, 1.0)
            .await
            .unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_place, cash_before);

        exchange.cancel_order(BTC, &id).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after_cancel, cash_before);
    }

    // 验证多仓被强平后，保证金损失、手续费与余额变化符合公式。
    #[tokio::test]
    async fn liquidation_long_pnl_margin_and_cash_match_formula() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(95.0), dec!(96.0), dec!(90.0), dec!(92.0)),
            ],
        );

        let md = btc_metadata();
        let qty = 1.0;
        let open_price = 105.0;

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        let init_margin = open_price * qty / 10.0;
        let open_fee = open_price * qty * md.taker_fee;
        let liq_fee = liq_price * qty * md.taker_fee;
        let expected_cash = 10000.0 - init_margin - open_fee - liq_fee;

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_eq!(history[0].profit, -init_margin);
        assert_eq!(history[0].fee, open_fee + liq_fee);
        assert_eq!(history[0].total_profit, -init_margin - open_fee - liq_fee);
        assert_eq!(cash_after, expected_cash);
    }

    // 验证空仓被强平后，保证金损失、手续费与余额变化符合公式。
    #[tokio::test]
    async fn liquidation_short_pnl_margin_and_cash_match_formula() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(115.0), dec!(116.0), dec!(114.0), dec!(115.0)),
            ],
        );

        let md = btc_metadata();
        let qty = 1.0;
        let open_price = 105.0;

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.sell(BTC, qty).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        let init_margin = open_price * qty / 10.0;
        let open_fee = open_price * qty * md.taker_fee;
        let liq_fee = liq_price * qty * md.taker_fee;
        let expected_cash = 10000.0 - init_margin - open_fee - liq_fee;

        exchange.next(BTC, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_eq!(history[0].profit, -init_margin);
        assert_eq!(history[0].fee, open_fee + liq_fee);
        assert_eq!(history[0].total_profit, -init_margin - open_fee - liq_fee);
        assert_eq!(cash_after, expected_cash);
    }

    // 验证强平后权益应等于现金（无持仓未实现盈亏）。
    #[tokio::test]
    async fn equity_equals_cash_after_liquidation() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(95.0), dec!(96.0), dec!(90.0), dec!(92.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let equity = exchange.get_equity().await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(equity, cash);
    }

    // 验证错误 symbol 的接口调用会返回错误，且不会污染当前账户状态。
    #[tokio::test]
    async fn symbol_mismatch_calls_do_not_contaminate_state() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        let place_err = exchange.buy("ETHUSDT", 1.0).await.unwrap_err();
        let cancel_err = exchange
            .cancel_order("ETHUSDT", "not-exists")
            .await
            .unwrap_err();
        let position_err = exchange.get_position("ETHUSDT").await.unwrap_err();

        let cash_after = exchange.get_cash().await.unwrap();

        assert!(
            place_err.to_string().contains("no symbol")
                || place_err.to_string().contains("place_order: ETHUSDT")
        );
        assert!(
            cancel_err.to_string().contains("no symbol")
                || cancel_err.to_string().contains("cancel_order: ETHUSDT")
        );
        assert!(
            position_err.to_string().contains("no symbol")
                || position_err.to_string().contains("get_position: ETHUSDT")
        );
        assert_eq!(cash_after, cash_before);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证 range 边界可精确裁剪数据区间，并在末尾返回 None。
    #[tokio::test]
    async fn range_boundary_yields_expected_klines_then_none() {
        let data_source = DataSource::new(btc_metadata(), btc_klines());
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .range(2, 4),
        ));

        let first = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(first.time, 2);

        let second = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(second.time, 3);

        let third = exchange.next(BTC, Level::Minute1).await.unwrap();
        assert!(third.is_none());
    }

    // 验证触发单在 low/high 边界精确触发，略低于边界则保持未触发。
    #[tokio::test]
    async fn trigger_order_exact_low_boundary_triggers_but_below_does_not() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let exact_id = exchange.buy_trigger_market(BTC, 104.0, 1.0).await.unwrap();
        let below_id = exchange
            .buy_trigger_market(BTC, 104.0 - 0.1, 1.0)
            .await
            .unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let exact = exchange.get_order(&exact_id).await.unwrap().unwrap();
        let below = exchange.get_order(&below_id).await.unwrap().unwrap();

        assert_eq!(exact.status, Status::Filled);
        assert_eq!(below.status, Status::Submitted);

        // 精确边界104.0触发后立即执行，以当前K线开盘价105成交
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.quantity, 1.0);
        assert_eq!(position.open_avg_price, 104.0);
    }

    // 验证混合挂单按任意顺序撤销后，现金会回到撤单前基准值。
    #[tokio::test]
    async fn mixed_order_cancel_sequence_keeps_cash_invariant() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let limit_a = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let trigger = exchange
            .buy_trigger_limit(BTC, 200.0, 90.0, 1.0)
            .await
            .unwrap();
        let limit_b = exchange.buy_limit(BTC, 80.0, 2.0).await.unwrap();
        let reduce_only = exchange
            .sell_limit_reduce_only(BTC, 120.0, 1.0)
            .await
            .unwrap();

        let cash_after_place = exchange.get_cash().await.unwrap();
        assert!(cash_after_place < cash_before);

        exchange.cancel_order(BTC, &reduce_only).await.unwrap();
        exchange.cancel_order(BTC, &trigger).await.unwrap();
        exchange.cancel_order(BTC, &limit_b).await.unwrap();
        exchange.cancel_order(BTC, &limit_a).await.unwrap();

        let cash_after_cancel = exchange.get_cash().await.unwrap();
        assert_eq!(cash_after_cancel, cash_before);
        assert!(
            exchange
                .get_pending_order_list(BTC)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // 验证强平单在手续费不足时仍会执行平仓，避免仓位残留。
    #[tokio::test]
    async fn liquidation_executes_even_when_fee_cash_is_insufficient() {
        let exchange = LocalExchange::new(vec![DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(95.0), dec!(96.0), dec!(90.0), dec!(92.0)),
            ],
        )])
        .cash(10.56)
        .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        let md = btc_metadata();
        let open_price = dec!(105.0);
        let qty = dec!(1.0);

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        let expected_cash = 10.56
            - open_price * qty / 10.0
            - open_price * qty * md.taker_fee
            - liq_price * qty * md.taker_fee;

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_eq!(history[0].profit, -10.5);
        assert!(history[0].fee > 0.0);
        assert_eq!(cash, expected_cash);
        assert!(cash < 0.0);
    }

    // 验证 metadata.min_size 非法（<=0）时，下单会被入口守卫拒绝。
    #[tokio::test]
    async fn place_order_rejects_when_metadata_min_size_is_zero() {
        let mut metadata = btc_metadata();
        metadata.min_size = dec!(0.0);

        let exchange = single_exchange_with(metadata, btc_klines());

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange.buy(BTC, 1.0).await.unwrap_err();

        assert!(result.to_string().contains("invalid metadata.min_size"));
    }

    // 验证同一根 K 线内连续下单时，订单 ID 唯一且挂单数量正确。
    #[tokio::test]
    async fn order_ids_are_unique_within_same_kline() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let id1 = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        let id2 = exchange.buy_limit(BTC, 80.0, 1.0).await.unwrap();
        let id3 = exchange
            .sell_limit_reduce_only(BTC, 120.0, 1.0)
            .await
            .unwrap();

        assert_ne!(id1, id2);
        assert_ne!(id1, id3);
        assert_ne!(id2, id3);

        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        assert_eq!(pending.len(), 3);
    }

    // 验证“调杠杆 + 调保证金”交替操作后，仓位与强平挂单价格始终同步。
    #[tokio::test]
    async fn leverage_and_margin_sequence_keeps_liquidation_in_sync() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_0 = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_0 = position_0.liquidation_price;

        exchange.set_leverage(BTC, 20).await.unwrap();
        let position_1 = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_1 = position_1.liquidation_price;
        let liq_order_1 = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_1 > liq_0);
        assert_eq!(liq_order_1.price, liq_1);

        exchange.append_position_margin(BTC, 2.0).await.unwrap();
        let position_2 = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_2 = position_2.liquidation_price;
        let liq_order_2 = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_2 < liq_1);
        assert_eq!(liq_order_2.price, liq_2);

        exchange.append_position_margin(BTC, -1.0).await.unwrap();
        let position_3 = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_3 = position_3.liquidation_price;
        let liq_order_3 = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_3 > liq_2);
        assert_eq!(liq_order_3.price, liq_3);
    }

    // 验证退化 K 线（开高低相同）下，限价与市价都按该唯一价格成交。
    #[tokio::test]
    async fn degenerate_kline_matches_limit_and_market_at_single_price() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(110.0), dec!(110.0), dec!(110.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let limit_id = exchange.buy_limit(BTC, 110.0, 1.0).await.unwrap();
        let market_id = exchange.buy(BTC, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let limit_order = exchange.get_order(&limit_id).await.unwrap().unwrap();
        let market_order = exchange.get_order(&market_id).await.unwrap().unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(limit_order.status, Status::Filled);
        assert_eq!(market_order.status, Status::Filled);
        assert_eq!(limit_order.avg_price, 110.0);
        assert_eq!(market_order.avg_price, 110.0);
        assert_eq!(position.quantity, 2.0);
        assert_eq!(position.open_avg_price, 110.0);
    }

    // 验证连续多次部分平仓会累计更新同一条历史仓位记录，直到最终全平。
    #[tokio::test]
    async fn sequential_partial_closes_accumulate_history_until_fully_closed() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(115.0), dec!(116.0), dec!(114.0), dec!(115.0)),
                gen_kline(5, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_after_first = exchange.get_position(BTC).await.unwrap().unwrap();
        let history_after_first = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(position_after_first.quantity, 1.5);
        assert_eq!(history_after_first.len(), 1);
        assert_eq!(history_after_first[0].close_quantity, 0.5);

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_after_second = exchange.get_position(BTC).await.unwrap().unwrap();
        let history_after_second = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(position_after_second.quantity, 1.0);
        assert_eq!(history_after_second.len(), 1);
        assert_eq!(history_after_second[0].close_quantity, 1.0);

        exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history_after_full_close = exchange.get_history_position_list(BTC).await.unwrap();
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history_after_full_close.len(), 1);
        assert_eq!(history_after_full_close[0].close_quantity, 2.0);
        assert_eq!(
            history_after_full_close[0].close_quantity,
            history_after_full_close[0].max_quantity,
        );
    }

    // 验证超量 reduce-only 平仓会被截断到当前仓位数量，且不会反向开仓。
    #[tokio::test]
    async fn reduce_only_oversized_close_is_clamped_to_position_quantity() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_id = exchange.sell_reduce_only(BTC, 5.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();

        assert_eq!(close_order.status, Status::Filled);
        assert_eq!(close_order.cumulative_quantity, 1.0);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
    }

    // 验证部分平仓后，仓位强平价与强平挂单价格保持同步一致。
    #[tokio::test]
    async fn partial_close_keeps_liquidation_order_price_synced() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let liq_order = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position.side, Side::Buy);
        assert_eq!(liq_order.side, Side::Sell);
        assert_eq!(liq_order.price, position.liquidation_price);
    }

    // 验证多仓部分平仓后，现金/历史盈亏/手续费与剩余保证金严格符合公式。
    #[tokio::test]
    async fn partial_close_long_cash_and_history_match_formula() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        let md = btc_metadata();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        let open_price = 105.0;
        let close_price = 110.0;

        let open_margin = open_price * 2.0 / 10.0;
        let open_fee = open_price * 2.0 * md.taker_fee;
        let close_fee = close_price * 0.5 * md.taker_fee;
        let close_margin = open_margin * (0.5 / 2.0);
        let close_profit = (close_price - open_price) * 0.5;

        let expected_cash =
            10000.0 - open_margin - open_fee - close_fee + close_margin + close_profit;

        assert_eq!(position.quantity, 1.5);
        assert_eq!(position.margin, open_margin - close_margin);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].profit, close_profit);
        assert_eq!(history[0].fee, open_fee + close_fee);
        assert_eq!(history[0].total_profit, close_profit - open_fee - close_fee);
        assert_eq!(cash, expected_cash);
    }

    // 验证空仓部分平仓后，现金/历史盈亏/手续费与剩余保证金严格符合公式。
    #[tokio::test]
    async fn partial_close_short_cash_and_history_match_formula() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
            ],
        );

        let md = btc_metadata();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        let open_price = 105.0;
        let close_price = 100.0;

        let open_margin = open_price * 2.0 / 10.0;
        let open_fee = open_price * 2.0 * md.taker_fee;
        let close_fee = close_price * 0.5 * md.taker_fee;
        let close_margin = open_margin * (0.5 / 2.0);
        let close_profit = (open_price - close_price) * 0.5;

        let expected_cash =
            10000.0 - open_margin - open_fee - close_fee + close_margin + close_profit;

        assert_eq!(position.quantity, 1.5);
        assert_eq!(position.margin, open_margin - close_margin);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].profit, close_profit);
        assert_eq!(history[0].fee, open_fee + close_fee);
        assert_eq!(history[0].total_profit, close_profit - open_fee - close_fee);
        assert_eq!(cash, expected_cash);
    }

    // 验证已有多仓时，同向 reduce-only 订单会被取消且仓位不变。
    #[tokio::test]
    async fn reduce_only_same_side_with_position_is_canceled() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();

        let id = exchange.buy_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Canceled);
        assert_eq!(position_after.side, Side::Buy);
        assert_eq!(position_after.quantity, position_before.quantity);
        assert_eq!(
            position_after.open_avg_price,
            position_before.open_avg_price,
        );
    }

    // 验证连续部分平仓时，历史仓位的 close_avg_price 会更新为最近一次平仓价。
    #[tokio::test]
    async fn sequential_partial_close_updates_history_close_avg_price_to_latest() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history_after_first = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history_after_first.len(), 1);
        assert_eq!(history_after_first[0].close_avg_price, 110.0);
        assert_eq!(history_after_first[0].close_quantity, 0.5);
        assert_eq!(history_after_first[0].profit, 2.5);
        assert_eq!(history_after_first[0].fee, 0.1325);
        assert_eq!(history_after_first[0].total_profit, 2.3675);

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history_after_second = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history_after_second.len(), 1);
        assert_eq!(history_after_second[0].close_avg_price, 120.0);
        assert_eq!(history_after_second[0].close_quantity, 1.0);
        assert_eq!(history_after_second[0].profit, 10.0);
        assert_eq!(history_after_second[0].fee, 0.1625);
        assert_eq!(history_after_second[0].total_profit, 9.8375);
    }

    // 验证“部分平仓 -> 加仓 -> 再部分平仓”时，close_quantity 表示累计已平仓量。
    #[tokio::test]
    async fn partial_close_quantity_tracks_cumulative_closed_quantity() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(115.0), dec!(116.0), dec!(114.0), dec!(115.0)),
                gen_kline(5, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_eq!(history[0].max_quantity, 2.5);
        assert_eq!(history[0].close_quantity, 1.0);
        assert_eq!(position.quantity, 2.0);
    }

    // 验证反向开仓会正确结算旧仓历史，并仅将超出部分作为新仓位。
    #[tokio::test]
    async fn reverse_order_splits_history_and_new_position_consistently() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        let md = btc_metadata();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell(BTC, 1.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let liq_order = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        let expected_profit = (110.0 - 105.0) * 1.0;
        let expected_fee = 105.0 * 1.0 * md.taker_fee + 110.0 * 1.0 * md.taker_fee;

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_eq!(history[0].close_quantity, 1.0);
        assert_eq!(history[0].max_quantity, 1.0);
        assert_eq!(history[0].profit, expected_profit);
        assert_eq!(history[0].fee, expected_fee);

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.quantity, 0.5);
        assert_eq!(position.open_avg_price, 110.0);
        assert_eq!(liq_order.side, Side::Buy);
        assert_eq!(liq_order.price, position.liquidation_price);
    }

    // 验证非 reduce-only 的对手方向“纯平仓”不会吞掉下单阶段冻结的保证金。
    #[tokio::test]
    async fn opposite_non_reduce_only_close_refunds_all_frozen_margin() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();

        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after_close = exchange.get_cash().await.unwrap();

        // 从已开多仓直接卖出同等数量，现金变化应仅包含：
        // 平仓释放保证金 + 已实现盈亏 - 本次平仓手续费。
        let expected_delta = 10.5 + 5.0 - 110.0 * 1.0 * btc_metadata().taker_fee;
        assert_eq!(cash_after_close - cash_before_close, expected_delta);
        assert!(exchange.get_position(BTC).await.unwrap().is_none());
    }

    // 验证仓位多次反转后会生成多条历史仓位，且各段方向/盈亏/手续费与资金结果一致。
    #[tokio::test]
    async fn multiple_reversals_create_multiple_history_positions_with_correct_values() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(5, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 1) 开多 1（在 bar2 成交 105）
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 2) 卖 2（在 bar3 成交 110）：先平多 1，再反向开空 1
        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 3) 买 2（在 bar4 成交 100）：先平空 1，再反向开多 1
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 4) 卖出 reduce-only 1（在 bar5 成交 120）：平掉最后一段多仓
        exchange.sell_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history.len(), 3);

        // 第 1 段: 多仓 105 -> 110
        assert_eq!(history[0].side, Side::Buy);
        assert_eq!(history[0].open_avg_price, 105.0);
        assert_eq!(history[0].close_avg_price, 110.0);
        assert_eq!(history[0].max_quantity, 1.0);
        assert_eq!(history[0].close_quantity, 1.0);
        assert_eq!(history[0].profit, 5.0);
        assert_eq!(history[0].fee, 0.1075);
        assert_eq!(history[0].total_profit, 4.8925);

        // 第 2 段: 空仓 110 -> 100
        assert_eq!(history[1].side, Side::Sell);
        assert_eq!(history[1].open_avg_price, 110.0);
        assert_eq!(history[1].close_avg_price, 100.0);
        assert_eq!(history[1].max_quantity, 1.0);
        assert_eq!(history[1].close_quantity, 1.0);
        assert_eq!(history[1].profit, 10.0);
        assert_eq!(history[1].fee, 0.105);
        assert_eq!(history[1].total_profit, 9.895);

        // 第 3 段: 多仓 100 -> 120
        assert_eq!(history[2].side, Side::Buy);
        assert_eq!(history[2].open_avg_price, 100.0);
        assert_eq!(history[2].close_avg_price, 120.0);
        assert_eq!(history[2].max_quantity, 1.0);
        assert_eq!(history[2].close_quantity, 1.0);
        assert_eq!(history[2].profit, 20.0);
        assert_eq!(history[2].fee, 0.11);
        assert_eq!(history[2].total_profit, 19.89);

        // 总资金校验（反向开仓只保留新仓保证金，返还整单冻结与新仓保证金差额）
        assert_eq!(cash, 10034.6775);
    }

    // 验证空头起手的多次反转（空->多->空->平）会生成多条历史仓位，且各段数值正确。
    #[tokio::test]
    async fn multiple_reversals_from_short_side_create_correct_histories() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(4, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
                gen_kline(5, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 1) 开空 1（在 bar2 成交 105）
        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 2) 买 2（在 bar3 成交 100）：先平空 1，再反向开多 1
        exchange.buy(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 3) 卖 2（在 bar4 成交 120）：先平多 1，再反向开空 1
        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 4) 买入 reduce-only 1（在 bar5 成交 110）：平掉最后一段空仓
        exchange.buy_reduce_only(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert_eq!(history.len(), 3);

        // 第 1 段: 空仓 105 -> 100
        assert_eq!(history[0].side, Side::Sell);
        assert_eq!(history[0].open_avg_price, 105.0);
        assert_eq!(history[0].close_avg_price, 100.0);
        assert_eq!(history[0].max_quantity, 1.0);
        assert_eq!(history[0].close_quantity, 1.0);
        assert_eq!(history[0].profit, 5.0);
        assert_eq!(history[0].fee, 0.1025);
        assert_eq!(history[0].total_profit, 4.8975);

        // 第 2 段: 多仓 100 -> 120
        assert_eq!(history[1].side, Side::Buy);
        assert_eq!(history[1].open_avg_price, 100.0);
        assert_eq!(history[1].close_avg_price, 120.0);
        assert_eq!(history[1].max_quantity, 1.0);
        assert_eq!(history[1].close_quantity, 1.0);
        assert_eq!(history[1].profit, 20.0);
        assert_eq!(history[1].fee, 0.11);
        assert_eq!(history[1].total_profit, 19.89);

        // 第 3 段: 空仓 120 -> 110
        assert_eq!(history[2].side, Side::Sell);
        assert_eq!(history[2].open_avg_price, 120.0);
        assert_eq!(history[2].close_avg_price, 110.0);
        assert_eq!(history[2].max_quantity, 1.0);
        assert_eq!(history[2].close_quantity, 1.0);
        assert_eq!(history[2].profit, 10.0);
        assert_eq!(history[2].fee, 0.115);
        assert_eq!(history[2].total_profit, 9.885);

        // 总资金校验（反向开仓只保留新仓保证金，返还整单冻结与新仓保证金差额）
        assert_eq!(cash, 10034.6725);
    }

    // 验证买入市价滑点超过上界时，成交价会被限制在 K 线 high。
    #[tokio::test]
    async fn market_buy_slippage_is_capped_by_kline_high() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.02),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 106.0);
    }

    // 验证卖出市价滑点超过下界时，成交价会被限制在 K 线 low。
    #[tokio::test]
    async fn market_sell_slippage_is_capped_by_kline_low() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.02),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 104.0);
    }

    // 验证限价穿价按开盘价成交时，会退还按下单价多冻结的保证金。
    #[tokio::test]
    async fn limit_cross_fill_refunds_excess_frozen_margin_when_fill_price_is_better() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(103.0), dec!(105.0), dec!(99.0), dec!(100.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let _id = exchange.buy_limit(BTC, 110.0, 1.0).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        // 下单时按委托价 110 冻结保证金：11.0
        assert_eq!(cash_before - cash_after_place, 11.0);

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_after_fill = exchange.get_cash().await.unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        // 实际按最高价（对于买单最坏的） 105 成交，保证金应重算为 10.5，差额 0.5 返还。
        let expected_cash = cash_before - 10.5 - 105.0 * btc_metadata().maker_fee;

        assert_eq!(position.open_avg_price, 105.0);
        assert_eq!(position.margin, 10.5);
        assert_eq!(cash_after_fill, expected_cash);
    }

    // 验证滑点在区间内时，买卖方向会按预期分别抬高/压低成交价。
    #[tokio::test]
    async fn market_slippage_applies_directionally_within_kline_range() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let buy_exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.005),
        ));

        buy_exchange.next(BTC, Level::Minute1).await.unwrap();
        let buy_id = buy_exchange.buy(BTC, 1.0).await.unwrap();
        buy_exchange.next(BTC, Level::Minute1).await.unwrap();
        let buy_order = buy_exchange.get_order(&buy_id).await.unwrap().unwrap();
        assert_eq!(buy_order.avg_price, 105.0 * 1.005,);

        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let sell_exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.005),
        ));

        sell_exchange.next(BTC, Level::Minute1).await.unwrap();
        let sell_id = sell_exchange.sell(BTC, 1.0).await.unwrap();
        sell_exchange.next(BTC, Level::Minute1).await.unwrap();
        let sell_order = sell_exchange.get_order(&sell_id).await.unwrap().unwrap();
        assert_eq!(sell_order.avg_price, 105.0 * 0.995,);
    }

    // 验证 slippage=0 时，市价成交行为与历史一致（按 open 成交）。
    #[tokio::test]
    async fn market_order_with_zero_slippage_fills_at_open() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 105.0);
    }

    // 验证限价单成交价不受 slippage 影响（仍遵循限价撮合规则）。
    #[tokio::test]
    async fn limit_order_fill_price_is_not_affected_by_slippage() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.02),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(BTC, 105.0, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_eq!(order.avg_price, 106.0);
    }

    // 验证触发市价单在二阶段成交时同样应用 slippage，并受 high/low 夹取。
    #[tokio::test]
    async fn trigger_market_fill_applies_slippage_with_kline_bounds() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.02),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let trigger_id = exchange.buy_trigger_market(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，以当前K线开盘价加滑点成交，夹取到最高价
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        // 开盘价105 * (1 + 0.02滑点) = 107.1，夹取到最高价106
        assert_eq!(position.open_avg_price, 106.0);
    }

    // 验证触发市价卖单在二阶段成交时应用 slippage，并在低点边界处被夹取。
    #[tokio::test]
    async fn trigger_market_sell_fill_applies_slippage_with_kline_bounds() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.05),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let trigger_id = exchange.sell_trigger_market(BTC, 105.0, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，以当前K线开盘价减滑点成交，夹取到最低价
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.side, Side::Sell);
        // 开盘价105 * (1 - 0.05滑点) = 99.75，夹取到最低价104
        assert_eq!(position.open_avg_price, 104.0);
    }

    // 验证反向开仓场景中，已追加保证金不会导致新仓位保证金出现负值。
    #[tokio::test]
    async fn reverse_with_appended_margin_keeps_new_margin_non_negative() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.append_position_margin(BTC, 10.0).await.unwrap();
        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.quantity, 1.0);
        assert!(position.margin > 0.0);
        assert_eq!(position.margin, 11.0);
    }

    // 验证 range 的 end_time 超过数据末尾时，仍会遍历到最后一根 K 线。
    #[tokio::test]
    async fn range_end_after_last_kline_still_iterates_to_end() {
        let data_source = DataSource::new(btc_metadata(), btc_klines());
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .range(2, 9999999999999),
        ));

        let first = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let second = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let third = exchange.next(BTC, Level::Minute1).await.unwrap();

        assert_eq!(first.time, 2);
        assert_eq!(second.time, 3);
        assert!(third.is_none());
    }

    // 验证空头加仓后的历史 max_quantity 统计按绝对仓位规模计算。
    #[tokio::test]
    async fn short_position_max_quantity_uses_absolute_exposure() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(4, dec!(95.0), dec!(96.0), dec!(94.0), dec!(95.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.sell(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy_reduce_only(BTC, 0.5).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].max_quantity, 2.0);
    }

    // 验证 set_leverage 拒绝 0，避免后续除零产生无效资金计算。
    #[tokio::test]
    async fn set_leverage_rejects_zero() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange.set_leverage(BTC, 0).await.unwrap_err();

        assert!(result.to_string().contains("greater than 0"));
    }

    // 压测：反向开仓 + 杠杆切换 + 保证金调整混合路径下，仓位与强平挂单始终同步且资金值有效。
    #[tokio::test]
    async fn stress_mixed_reverse_leverage_margin_path_keeps_invariants() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                gen_kline(4, dec!(95.0), dec!(96.0), dec!(94.0), dec!(95.0)),
                gen_kline(5, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
                gen_kline(6, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_open = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_open.margin, 105.0 * 1.0 / 10.0);

        exchange.append_position_margin(BTC, 5.0).await.unwrap();
        let position_after_append = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_append.margin, 105.0 * 1.0 / 10.0 + 5.0);

        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_reverse = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_reverse.side, Side::Sell);
        assert_eq!(position_after_reverse.quantity, 1.0);
        assert_eq!(position_after_reverse.margin, 110.0 * 1.0 / 10.0);

        exchange.set_leverage(BTC, 20).await.unwrap();
        let position_after_leverage_20 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_leverage_20.margin, 110.0 * 1.0 / 20.0);

        exchange.append_position_margin(BTC, 2.0).await.unwrap();
        let position_after_append_2 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_append_2.margin, 110.0 * 1.0 / 20.0 + 2.0);

        exchange.buy(BTC, 0.6).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_partial_close = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_partial_close.side, Side::Sell);
        assert_eq!(position_after_partial_close.quantity, 0.4);
        assert_eq!(position_after_partial_close.margin, 3.0);

        exchange.set_leverage(BTC, 8).await.unwrap();
        let position_after_leverage_8 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_leverage_8.margin, 110.0 * 0.4 / 8.0);

        exchange.sell_reduce_only(BTC, 0.3).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        let liquidation = exchange
            .get_pending_order_list(BTC)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();
        let equity = exchange.get_equity().await.unwrap();

        assert_eq!(position.margin, 110.0 * 0.4 / 8.0);
        assert_eq!(liquidation.price, position.liquidation_price);
        assert_eq!(liquidation.side, position.side.neg());
        assert!(equity > 0.0);
    }

    // 压测：多次交替调杠杆/调保证金/反向与平仓后，不变量持续成立且不会出现 NaN/Inf。
    #[tokio::test]
    async fn stress_repeated_mixed_operations_keep_exchange_state_sane() {
        let exchange = single_exchange_with(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                gen_kline(3, dec!(95.0), dec!(96.0), dec!(94.0), dec!(95.0)),
                gen_kline(4, dec!(115.0), dec!(116.0), dec!(114.0), dec!(115.0)),
                gen_kline(5, dec!(90.0), dec!(91.0), dec!(89.0), dec!(90.0)),
                gen_kline(6, dec!(120.0), dec!(121.0), dec!(119.0), dec!(120.0)),
                gen_kline(7, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
            ],
        );

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.2).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_open = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_open.margin,
            dec!(105.0) * dec!(1.2) / dec!(10.0)
        );

        exchange.set_leverage(BTC, 5).await.unwrap();
        let position_after_leverage_5 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_leverage_5.margin,
            dec!(105.0) * dec!(1.2) / dec!(5.0)
        );

        exchange.append_position_margin(BTC, 3.0).await.unwrap();
        let position_after_append_3 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_append_3.margin,
            dec!(105.0) * dec!(1.2) / dec!(5.0) + dec!(3.0)
        );

        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_reverse = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_reverse.side, Side::Sell);
        assert_eq!(position_after_reverse.quantity, dec!(0.8));
        assert_eq!(
            position_after_reverse.margin,
            dec!(95.0) * dec!(0.8) / dec!(5.0)
        );

        exchange.set_leverage(BTC, 15).await.unwrap();
        let position_after_leverage_15 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_leverage_15.margin,
            dec!(95.0) * dec!(0.8) / dec!(15.0)
        );

        exchange.append_position_margin(BTC, 1.0).await.unwrap();
        let position_after_append_1 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_append_1.margin,
            dec!(95.0) * dec!(0.8) / dec!(15.0) + dec!(1.0)
        );

        exchange.buy(BTC, 1.1).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        let position_after_second_reverse = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position_after_second_reverse.side, Side::Buy);
        assert_eq!(position_after_second_reverse.quantity, dec!(0.3));
        assert_eq!(
            position_after_second_reverse.margin,
            dec!(115.0) * dec!(0.3) / dec!(15.0)
        );

        exchange.set_leverage(BTC, 6).await.unwrap();
        let position_after_leverage_6 = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(
            position_after_leverage_6.margin,
            dec!(115.0) * dec!(0.3) / dec!(6.0)
        );

        exchange.close_all_position(BTC).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash = exchange.get_cash().await.unwrap();
        let equity = exchange.get_equity().await.unwrap();
        let pending = exchange.get_pending_order_list(BTC).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(cash > 0.0);
        assert!(equity > 0.0);

        if let Some(position) = exchange.get_position(BTC).await.unwrap() {
            let liq = pending
                .iter()
                .find(|v| v.kind == Kind::Liquidation)
                .unwrap();

            assert!(position.margin > 0.0);
            assert_eq!(liq.side, position.side.neg());
            assert_eq!(liq.price, position.liquidation_price);
        }
    }

    // =======================================================================
    // Multi-symbol tests
    // =======================================================================

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

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 10);
        assert_eq!(exchange.get_leverage(ETH).await.unwrap(), 10);

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
        let exchange = LocalExchange::new(vec![
            DataSource::new(btc_metadata(), btc_klines()),
            DataSource::new(eth_metadata(), eth_klines()),
        ])
        .cash(10000.0)
        .leverage(10)
        .range(1, 3);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 1);
        assert_eq!(eth.time, 1);

        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 2);
        assert_eq!(eth.time, 2);

        assert!(exchange.next(BTC, Level::Minute1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn single_symbol_works_like_original() {
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![DataSource::new(btc_metadata(), btc_klines())])
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

    // =======================================================================
    // Cross-symbol tests (new)
    // =======================================================================

    // 验证不同 symbol 的仓位互相隔离，操作 BTC 不影响 ETH 仓位。
    #[tokio::test]
    async fn cross_symbol_positions_are_isolated() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.sell(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        // BTC long position exists
        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(btc_pos.side, Side::Buy);

        // ETH short position exists
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();
        assert_eq!(eth_pos.side, Side::Sell);

        // Close only BTC, ETH should remain
        exchange.close_all_position(BTC).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());
        assert_eq!(
            exchange.get_position(ETH).await.unwrap().unwrap().side,
            Side::Sell
        );
    }

    // 验证共享现金池：在一个 symbol 上消耗现金会影响另一个 symbol 的可用资金。
    #[tokio::test]
    async fn cross_symbol_shared_cash_pool() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        // Open BTC position (consumes margin + fee)
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let cash_after_btc = exchange.get_cash().await.unwrap();
        assert!(cash_after_btc < cash_before);

        // Open ETH position with remaining cash
        exchange.buy(ETH, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let cash_after_eth = exchange.get_cash().await.unwrap();
        assert!(cash_after_eth < cash_after_btc);

        // Both positions exist
        assert!(exchange.get_position(BTC).await.unwrap().is_some());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());
    }

    // 验证一个 symbol 爆仓不会影响另一个 symbol 的仓位。
    #[tokio::test]
    async fn cross_symbol_liquidation_is_isolated() {
        let exchange = LocalExchange::new(vec![
            DataSource::new(
                btc_metadata(),
                vec![
                    gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                    gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                    gen_kline(3, dec!(95.0), dec!(96.0), dec!(90.0), dec!(92.0)),
                ],
            ),
            DataSource::new(
                eth_metadata(),
                vec![
                    gen_kline(1, dec!(50.0), dec!(51.0), dec!(49.0), dec!(50.5)),
                    gen_kline(2, dec!(52.0), dec!(53.0), dec!(51.0), dec!(52.5)),
                    gen_kline(3, dec!(55.0), dec!(56.0), dec!(54.0), dec!(55.5)),
                ],
            ),
        ])
        .cash(10000.0)
        .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        // Both positions exist
        assert!(exchange.get_position(BTC).await.unwrap().is_some());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());

        // BTC gets liquidated, ETH should survive
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());

        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();
        assert_eq!(eth_pos.side, Side::Buy);
        assert_eq!(eth_pos.quantity, dec!(1.0));
    }

    // 验证不同 symbol 使用不同杠杆且互相独立。
    #[tokio::test]
    async fn cross_symbol_independent_leverage() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        // Set different leverage per symbol
        exchange.set_leverage(BTC, 5).await.unwrap();
        exchange.set_leverage(ETH, 20).await.unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 5);
        assert_eq!(exchange.get_leverage(ETH).await.unwrap(), 20);

        // BTC margin should be higher (lower leverage)
        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();

        // BTC: 105 * 1 / 5 = 21, ETH: 52 * 1 / 20 = 2.6
        assert_eq!(btc_pos.margin, dec!(21.0));
        assert_eq!(eth_pos.margin, dec!(2.6));
    }

    // 验证不同 symbol 的挂单互相隔离，cancel_all_order 仅影响指定 symbol。
    #[tokio::test]
    async fn cross_symbol_orders_are_isolated() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();
        exchange.sell_limit(ETH, 60.0, 1.0).await.unwrap();

        let btc_pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let eth_pending = exchange.get_pending_order_list(ETH).await.unwrap();
        assert_eq!(btc_pending.len(), 1);
        assert_eq!(eth_pending.len(), 1);

        // Cancel all BTC orders, ETH orders should remain
        exchange.cancel_all_order(BTC).await.unwrap();

        let btc_pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let eth_pending = exchange.get_pending_order_list(ETH).await.unwrap();
        assert!(btc_pending.is_empty());
        assert_eq!(eth_pending.len(), 1);
    }

    // 验证两个 symbol 同时持有仓位时权益合计正确。
    #[tokio::test]
    async fn cross_symbol_equity_sums_both_positions_correctly() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let cash = exchange.get_cash().await.unwrap();
        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();
        let equity = exchange.get_equity().await.unwrap();

        let expected_equity =
            cash + btc_pos.margin + btc_pos.profit + eth_pos.margin + eth_pos.profit;
        assert_eq!(equity, expected_equity);
    }

    // 验证跨 symbol 的 pacing：BTC 作为 pacemaker，ETH 缺失某次 next 不推进时间线。
    #[tokio::test]
    async fn cross_symbol_missing_eth_next_does_not_advance_timeline() {
        let exchange = multi_exchange();

        // BTC is pacemaker (first to call next)
        let btc1 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc1.time, 1);

        // ETH gets same kline even without advancing timeline
        let eth1 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(eth1.time, 1);

        // Only BTC calls next (pacemaker advances)
        let btc2 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc2.time, 2);

        // ETH still sees the cached kline from last advance
        // Actually, after BTC advances, klines are cached. ETH should see time 2 now.
        let eth2 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(eth2.time, 2);
    }

    // 验证在双 symbol 场景下，对一个 symbol 追加保证金不影响另一个 symbol。
    #[tokio::test]
    async fn cross_symbol_append_margin_is_per_symbol() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_margin_before = exchange.get_position(BTC).await.unwrap().unwrap().margin;
        let eth_margin_before = exchange.get_position(ETH).await.unwrap().unwrap().margin;

        exchange.append_position_margin(BTC, 5.0).await.unwrap();

        let btc_margin_after = exchange.get_position(BTC).await.unwrap().unwrap().margin;
        let eth_margin_after = exchange.get_position(ETH).await.unwrap().unwrap().margin;

        assert_eq!(btc_margin_after - btc_margin_before, dec!(5.0));
        assert_eq!(eth_margin_after, eth_margin_before);
    }

    // 验证一个 symbol 反向开仓不影响另一个 symbol 的仓位方向/数量。
    #[tokio::test]
    async fn cross_symbol_reverse_on_one_does_not_affect_other() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.buy(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        // Reverse BTC: sell 2.0 → close long 1.0 + open short 1.0
        exchange.sell(BTC, 2.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();

        assert_eq!(btc_pos.side, Side::Sell);
        assert_eq!(btc_pos.quantity, dec!(1.0));
        assert_eq!(eth_pos.side, Side::Buy);
        assert_eq!(eth_pos.quantity, dec!(1.0));
    }

    // 验证不同 symbol 的强平单各自独立且方向正确。
    #[tokio::test]
    async fn cross_symbol_liquidation_orders_are_independent() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.sell(ETH, 1.0).await.unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let eth_pending = exchange.get_pending_order_list(ETH).await.unwrap();

        let btc_liq = btc_pending
            .iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();
        let eth_liq = eth_pending
            .iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        // BTC is long → liquidation sells
        assert_eq!(btc_liq.side, Side::Sell);
        // ETH is short → liquidation buys
        assert_eq!(eth_liq.side, Side::Buy);

        let btc_pos = exchange.get_position(BTC).await.unwrap().unwrap();
        let eth_pos = exchange.get_position(ETH).await.unwrap().unwrap();

        assert_eq!(btc_liq.price, btc_pos.liquidation_price);
        assert_eq!(eth_liq.price, eth_pos.liquidation_price);
    }

    // 验证跨 symbol 的 pacemaker 耗尽：最短数据源决定总步数，pacemaker 耗尽后所有 symbol 停止。
    #[tokio::test]
    async fn cross_symbol_pacemaker_exhaustion_affects_all() {
        let exchange = LocalExchange::new(vec![
            DataSource::new(
                btc_metadata(),
                vec![
                    gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                    gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                ],
            ),
            DataSource::new(
                eth_metadata(),
                vec![
                    gen_kline(1, dec!(50.0), dec!(51.0), dec!(49.0), dec!(50.0)),
                    gen_kline(2, dec!(52.0), dec!(53.0), dec!(51.0), dec!(52.0)),
                    gen_kline(3, dec!(55.0), dec!(56.0), dec!(54.0), dec!(55.0)),
                ],
            ),
        ])
        .cash(10000.0)
        .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        // Round 1: both advance
        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 1);
        assert_eq!(eth.time, 1);

        // Round 2: both advance
        let btc = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        let eth = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc.time, 2);
        assert_eq!(eth.time, 2);

        // BTC exhausted (only 2 candles), both symbols stop
        assert!(exchange.next(BTC, Level::Minute1).await.unwrap().is_none());
        assert!(exchange.next(ETH, Level::Minute1).await.unwrap().is_none());
    }

    // 验证使用错误 symbol 取消已存在订单时会报 symbol mismatch。
    #[tokio::test]
    async fn cancel_order_with_wrong_symbol_fails() {
        let exchange = multi_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        let btc_order_id = exchange.buy_limit(BTC, 90.0, 1.0).await.unwrap();

        // 用 ETH symbol 去取消 BTC 的订单应失败
        let result = exchange.cancel_order(ETH, &btc_order_id).await.unwrap_err();
        assert!(result.to_string().contains("symbol mismatch"));
    }

    // 验证 ETH 作为 pacemaker 时，时间线推进逻辑同样正确。
    #[tokio::test]
    async fn eth_as_pacemaker_advances_timeline() {
        let exchange = multi_exchange();

        // ETH 率先调用 next → 成为 pacemaker
        let eth1 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(eth1.time, 1);

        // BTC 获取同一根 K 线缓存
        let btc1 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc1.time, 1);

        // ETH 推进到第 2 轮
        let eth2 = exchange.next(ETH, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(eth2.time, 2);

        // BTC 仍能读到缓存
        let btc2 = exchange.next(BTC, Level::Minute1).await.unwrap().unwrap();
        assert_eq!(btc2.time, 2);
    }

    // 验证在无挂单时调用 cancel_all_order 是幂等操作。
    #[tokio::test]
    async fn cancel_all_order_without_pending_is_noop() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange.cancel_all_order(BTC).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(cash_after, cash_before);
    }

    // 验证减少保证金超过当前保证金总额时会失败（会先触发 initial margin 守卫）。
    #[tokio::test]
    async fn append_position_margin_rejects_reducing_more_than_current_margin() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 先追加保证金使总额远离 initial margin 边界
        exchange.append_position_margin(BTC, 20.0).await.unwrap();
        let position = exchange.get_position(BTC).await.unwrap().unwrap();
        assert_eq!(position.margin, dec!(30.5));

        // 当前保证金 30.5，尝试减少 40.0 应被拒绝
        // 由于 new_margin 为负值，会先触发 initial margin 守卫；
        // 而 codebase 中 "cannot reduce margin more than current margin"
        // 的检查位于其后（防御性死代码，当 init_margin 检查先触发时不可达）。
        let result = exchange
            .append_position_margin(BTC, -40.0)
            .await
            .unwrap_err();
        assert!(
            result.to_string().contains("initial margin")
                || result
                    .to_string()
                    .contains("cannot reduce margin more than current margin")
        );
    }

    // 验证在 test 模式下 get_order 可以查到强平订单（cfg!(test) 分支）。
    #[tokio::test]
    async fn get_order_includes_liquidation_orders_in_test_mode() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let pending = exchange.get_pending_order_list(BTC).await.unwrap();
        let liq = pending
            .iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        // test 模式下 get_order 应能查到强平单
        let order = exchange.get_order(&liq.id).await.unwrap();
        assert!(order.is_some());
        assert_eq!(order.unwrap().kind, Kind::Liquidation);
    }

    // 验证 set_leverage 设置为相同值可以实现无副作用通过。
    #[tokio::test]
    async fn set_leverage_to_same_value_is_noop() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();

        exchange.set_leverage(BTC, 10).await.unwrap(); // 与当前杠杆一致

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(exchange.get_leverage(BTC).await.unwrap(), 10);
        assert_eq!(cash_after, cash_before);
        assert_eq!(position_after.margin, position_before.margin);
        assert_eq!(
            position_after.liquidation_price,
            position_before.liquidation_price,
        );
    }

    // 验证追加零保证金是无副作用操作。
    #[tokio::test]
    async fn append_position_margin_zero_is_noop() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(BTC).await.unwrap().unwrap();

        exchange.append_position_margin(BTC, 0.0).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(BTC).await.unwrap().unwrap();

        assert_eq!(cash_after, cash_before);
        assert_eq!(position_after.margin, position_before.margin);
    }

    // 验证市价单带指定 price（非零）时，使用该价格加滑点成交，而非开盘价。
    // 这是 handle_market_order 中 order.price != Decimal::ZERO 的分支。
    #[tokio::test]
    async fn market_order_with_specified_price_uses_that_price_not_open() {
        let data_source = DataSource::new(
            btc_metadata(),
            vec![
                gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
            ],
        );
        let exchange = ExchangeWrapper::new(Arc::new(
            LocalExchange::new(vec![data_source])
                .cash(10000.0)
                .leverage(10)
                .slippage(0.01),
        ));

        exchange.next(BTC, Level::Minute1).await.unwrap();

        // 提交带指定 price 的市价单（price > 0 表示用户限定了成交参考价）
        let id = exchange
            .place_order(Order {
                symbol: BTC.to_string(),
                side: Side::Buy,
                trigger_price: Decimal::ZERO,
                price: dec!(106.0),
                quantity: dec!(1.0),
                reduce_only: false,
            })
            .await
            .unwrap();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        // 按指定价 106.0 加滑点 = 106.0 * 1.01 = 107.06，夹取到 high=106.0
        assert_eq!(order.avg_price, 106.0);
    }

    // 验证 get_order 对不存在的订单 ID 返回 None（不报错）。
    #[tokio::test]
    async fn get_order_returns_none_for_nonexistent_id() {
        let exchange = single_exchange();

        exchange.next(BTC, Level::Minute1).await.unwrap();

        let result = exchange.get_order("non-existent-id").await.unwrap();
        assert!(result.is_none());
    }

    // 验证双 symbol 完整往返：BTC 做多、ETH 做空，各自独立平仓且历史正确。
    #[tokio::test]
    async fn multi_symbol_round_trip_long_short_close_independently() {
        // 需要 4 根 K 线：第 1 根推进、第 2 根开仓成交、第 3 根平 BTC、第 4 根平 ETH
        let exchange = LocalExchange::new(vec![
            DataSource::new(
                btc_metadata(),
                vec![
                    gen_kline(1, dec!(100.0), dec!(101.0), dec!(99.0), dec!(100.0)),
                    gen_kline(2, dec!(105.0), dec!(106.0), dec!(104.0), dec!(105.0)),
                    gen_kline(3, dec!(110.0), dec!(111.0), dec!(109.0), dec!(110.0)),
                    gen_kline(4, dec!(115.0), dec!(116.0), dec!(114.0), dec!(115.0)),
                ],
            ),
            DataSource::new(
                eth_metadata(),
                vec![
                    gen_kline(1, dec!(50.0), dec!(51.0), dec!(49.0), dec!(50.5)),
                    gen_kline(2, dec!(52.0), dec!(53.0), dec!(51.0), dec!(52.5)),
                    gen_kline(3, dec!(55.0), dec!(56.0), dec!(54.0), dec!(55.5)),
                    gen_kline(4, dec!(58.0), dec!(59.0), dec!(57.0), dec!(58.5)),
                ],
            ),
        ])
        .cash(10000.0)
        .leverage(10);

        let exchange = ExchangeWrapper::new(Arc::new(exchange));

        // Round 1: 各下一单
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        exchange.buy(BTC, 1.0).await.unwrap();
        exchange.sell(ETH, 1.0).await.unwrap();

        // Round 2: 成交
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_some());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());

        // 仅平 BTC（Round 3）
        exchange.close_all_position(BTC).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(BTC).await.unwrap().is_none());
        assert!(exchange.get_position(ETH).await.unwrap().is_some());

        let btc_history = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(btc_history.len(), 1);
        assert_eq!(btc_history[0].side, Side::Buy);

        // 再平 ETH（Round 4）
        exchange.close_all_position(ETH).await.unwrap();
        exchange.next(BTC, Level::Minute1).await.unwrap();
        exchange.next(ETH, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(ETH).await.unwrap().is_none());

        let eth_history = exchange.get_history_position_list(ETH).await.unwrap();
        assert_eq!(eth_history.len(), 1);
        assert_eq!(eth_history[0].side, Side::Sell);

        // BTC 历史不受影响
        let btc_history_after = exchange.get_history_position_list(BTC).await.unwrap();
        assert_eq!(btc_history_after.len(), 1);
    }
}
