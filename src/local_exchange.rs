use anyhow::Context;
use anyhow::bail;
use indexmap::IndexMap;
use inherits::inherits;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::*;
use crate::exchange::*;
use crate::order::*;
use crate::util::*;

#[derive(Clone)]
pub struct LocalExchange {
    inner: Arc<Mutex<LocalExchangeInner>>,
}

struct LocalExchangeInner {
    data_source: DataSource,
    kline: KLine,
    index: usize,
    cash: f64,
    leverage: u32,
    slippage: f64,
    history_order_list: IndexMap<String, OrderEx>,
    pending_order_list: IndexMap<String, OrderEx>,
    position: Option<PositionEx>,
    history_position_list: Vec<HistoryPosition>,
    start_index: usize,
    end_index: usize,
    id: u64,
}

#[inherits(Order)]
#[derive(Clone)]
struct OrderEx {
    id: String,
    kind: Kind,
    avg_price: f64,
    cumulative_quantity: f64,
    create_time: u64,
    update_time: u64,
    status: Status,
    freeze_margin: f64,
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

impl LocalExchange {
    pub fn new(data_source: DataSource) -> Self {
        let end_index = data_source.data.len().saturating_sub(1);

        Self {
            inner: Arc::new(Mutex::new(LocalExchangeInner {
                data_source,
                kline: KLine::default(),
                index: 0,
                cash: 10000.0,
                leverage: 1,
                slippage: 0.0,
                position: None,
                history_order_list: IndexMap::new(),
                pending_order_list: IndexMap::new(),
                history_position_list: Vec::new(),
                start_index: 0,
                end_index,
                id: 0,
            })),
        }
    }

    pub fn cash(self, cash: f64) -> Self {
        self.inner.try_lock().unwrap().cash = if cash.is_finite() { cash.max(0.0) } else { 0.0 };
        self
    }

    pub fn leverage(self, leverage: u32) -> Self {
        self.inner.try_lock().unwrap().leverage = leverage.max(1);
        self
    }

    pub fn slippage(self, slippage: f64) -> Self {
        let slippage = if slippage.is_finite() {
            slippage.max(0.0)
        } else {
            0.0
        };

        self.inner.try_lock().unwrap().slippage = slippage;

        self
    }

    pub fn range(self, start_time: u64, end_time: u64) -> Self {
        let mut inner = self.inner.try_lock().unwrap();

        inner.start_index = inner
            .data_source
            .data
            .iter()
            .position(|v| v.time >= start_time)
            .unwrap_or(0);

        inner.end_index = inner
            .data_source
            .data
            .iter()
            .position(|v| v.time >= end_time)
            .unwrap_or(inner.data_source.data.len().saturating_sub(1));

        drop(inner);

        self
    }
}

impl LocalExchangeInner {
    fn calc_market_price_slippage(&self, side: Side, market_price: f64) -> f64 {
        let mut price = match side {
            Side::Buy => market_price * (1.0 + self.slippage),
            Side::Sell => market_price * (1.0 - self.slippage),
        }
        .snap_to_tick(self.data_source.metadata.tick_size);

        let low = self.kline.low.min(self.kline.high);
        let high = self.kline.low.max(self.kline.high);

        price = price.clamp(low, high);

        price
    }

    fn freeze_margin(&mut self, order: &mut OrderEx, leverage: u32) -> anyhow::Result<()> {
        let freeze_margin = calc_initial_margin(order.price, order.quantity, leverage);

        self.need_cash(freeze_margin)?;

        order.freeze_margin = freeze_margin;

        Ok(())
    }

    fn freeze_margin_avg_price(
        &mut self,
        order: &mut OrderEx,
        leverage: u32,
    ) -> anyhow::Result<()> {
        if order.reduce_only || order.freeze_margin == 0.0 {
            return Ok(());
        }

        let precision = 1e-8;
        let filled_margin = calc_initial_margin(order.avg_price, order.quantity, leverage);

        if order.freeze_margin.snap_gt(filled_margin, precision) {
            self.cash += order.freeze_margin - filled_margin;
        } else if order.freeze_margin.snap_lt(filled_margin, precision) {
            self.need_cash(filled_margin - order.freeze_margin)?;
        }

        order.freeze_margin = filled_margin;

        Ok(())
    }

    fn need_cash(&mut self, need: f64) -> anyhow::Result<()> {
        let precision = 1e-8;

        if !need.is_zero(precision) && self.cash.snap_lt(need, precision) {
            bail!("cash shortage, need {}, balance {}", need, self.cash);
        }

        self.cash -= need;

        Ok(())
    }

    fn place_order(&mut self, order: Order, kind: Kind) -> anyhow::Result<String> {
        let leverage = self.leverage;

        let id = t2s(self.kline.time) + " [" + self.id.to_string().as_str() + "]";

        self.id += 1;

        let mut order = OrderEx {
            parent: order,
            id: id.clone(),
            kind,
            avg_price: 0.0,
            cumulative_quantity: 0.0,
            create_time: self.kline.time,
            update_time: self.kline.time,
            status: Status::Submitted,
            freeze_margin: 0.0,
        };

        if !order.reduce_only && order.is_limit() {
            // 对于市价单，保证金在成交时被冻结
            // 对于计划委托单，保证金在触发时被冻结
            // 只有在不是只减仓订单的情况下，我们才需要冻结保证金
            // 这里按整单先冻结；若后续仅平仓或发生反向开仓，
            // 会在成交分支按实际需要返还多冻结保证金。
            self.freeze_margin(&mut order, leverage)
                .context(format!("place_order: {}", order.symbol))?;
        }

        self.pending_order_list.insert(id.clone(), order);

        Ok(id)
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

        if let Some(v) = &mut self.position {
            v.profit = if v.side == Side::Buy {
                (self.kline.close - v.open_avg_price) * v.quantity
            } else {
                (v.open_avg_price - self.kline.close) * v.quantity
            };

            v.liquidation_price = calc_liquidation_price(
                v.leverage,
                self.data_source.metadata.maintenance,
                v.side,
                v.open_avg_price,
                v.quantity,
                v.margin,
            );
        }
    }

    fn update_order(&mut self, order_id: &str, order_queue: &mut VecDeque<String>) {
        let Some(order) = self.pending_order_list.get(order_id) else {
            return;
        };

        if order.status != Status::Submitted {
            return;
        }

        if order.kind == Kind::Trigger {
            // 条件单触发判断
            if !(order.trigger_price >= self.kline.low && order.trigger_price <= self.kline.high) {
                return;
            }

            let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
                Some(v) => v,
                None => return,
            };

            let result = self.place_order(
                Order {
                    symbol: order_ref.symbol.clone(),
                    side: order_ref.side,
                    trigger_price: 0.0,
                    price: if order_ref.price == 0.0 {
                        order_ref.trigger_price
                    } else {
                        order_ref.price
                    },
                    quantity: order_ref.quantity,
                    reduce_only: order_ref.reduce_only,
                },
                if order_ref.price == 0.0 {
                    Kind::Market
                } else {
                    Kind::Limit
                },
            );

            order_ref.status = if let Ok(v) = result {
                order_queue.push_back(v);
                Status::Filled
            } else {
                Status::Rejected
            };

            self.history_order_list
                .insert(order_ref.id.clone(), order_ref);
        } else if order.kind == Kind::Limit || order.kind == Kind::Liquidation {
            // 限价单逻辑（包含强平单的处理）
            let order_ref = if order.kind == Kind::Liquidation {
                if !(order.price >= self.kline.low && order.price <= self.kline.high) {
                    return;
                }

                let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
                    Some(v) => v,
                    None => return,
                };

                order_ref.avg_price = order_ref.price;
                order_ref
            } else if (order.side == Side::Buy && order.price >= self.kline.open)
                || (order.side == Side::Sell && order.price <= self.kline.open)
            {
                let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
                    Some(v) => v,
                    None => return,
                };

                order_ref.avg_price = if order_ref.side == Side::Buy {
                    self.kline.high
                } else {
                    self.kline.low
                };

                order_ref
            } else if (order.side == Side::Buy && self.kline.low <= order.price)
                || (order.side == Side::Sell && self.kline.high >= order.price)
            {
                let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
                    Some(v) => v,
                    None => return,
                };

                order_ref.avg_price = order_ref.price;
                order_ref
            } else {
                return;
            };

            let fee_rate = if order_ref.kind == Kind::Liquidation {
                self.data_source.metadata.taker_fee
            } else {
                self.data_source.metadata.maker_fee
            };

            self.execute_order(order_id, order_ref, fee_rate);
        } else {
            // 市价单
            let mut order_ref = match self.pending_order_list.shift_remove(order_id) {
                Some(v) => v,
                None => return,
            };

            if order_ref.price == 0.0 {
                order_ref.price = self.calc_market_price_slippage(order_ref.side, self.kline.open);
            } else {
                // 条件市价单的市价等于触发价
                order_ref.price = self.calc_market_price_slippage(order_ref.side, order_ref.price);
            }

            order_ref.avg_price = order_ref.price;
            self.execute_order(order_id, order_ref, self.data_source.metadata.taker_fee);
        }
    }

    fn execute_order(&mut self, id: &str, mut order_ref: OrderEx, fee_rate: f64) {
        order_ref.update_time = self.kline.time;

        if order_ref.reduce_only {
            if let Some(v) = &self.position {
                if v.side == order_ref.side {
                    order_ref.status = Status::Canceled;
                    self.history_order_list
                        .insert(order_ref.id.clone(), order_ref);
                    return;
                } else {
                    let close_quantity = order_ref.quantity.min(v.quantity);

                    order_ref.quantity = close_quantity;
                }
            } else {
                order_ref.status = Status::Canceled;
                order_ref.update_time = self.kline.time;
                self.history_order_list
                    .insert(order_ref.id.clone(), order_ref);
                return;
            }
        } else if order_ref.freeze_margin == 0.0
            && self.freeze_margin(&mut order_ref, self.leverage).is_err()
        {
            // 市价单冻结保证金
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref);
            return;
        }

        // 退还超额冻结的保证金
        // 例如，做多委托价 ≥ 市价时，将导致订单立即以市价成交
        if !order_ref
            .avg_price
            .snap_eq(order_ref.price, self.data_source.metadata.tick_size)
            && self
                .freeze_margin_avg_price(&mut order_ref, self.leverage)
                .is_err()
        {
            self.cash += order_ref.freeze_margin;
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref);
            return;
        }

        // 检查市价单和限价单名义价值
        if order_ref.kind.is_normal()
            && self.data_source.metadata.min_notional != 0.0
            && (order_ref.avg_price * order_ref.quantity).snap_lt(
                self.data_source.metadata.min_notional,
                self.data_source.metadata.tick_size,
            )
        {
            self.cash += order_ref.freeze_margin;
            order_ref.status = Status::Rejected;
            self.history_order_list
                .insert(order_ref.id.clone(), order_ref);
            return;
        }

        let fee_cost = order_ref.avg_price * order_ref.quantity * fee_rate;

        if self.need_cash(fee_cost).is_err() {
            if order_ref.kind.is_normal() {
                // 普通单（包括 reduce-only）不允许因手续费导致现金为负，余额不足直接拒绝。
                self.cash += order_ref.freeze_margin;
                order_ref.status = Status::Rejected;
                self.history_order_list
                    .insert(order_ref.id.clone(), order_ref);
                return;
            } else {
                // 强平/ADL 允许穿仓，但手续费仍需入账，现金可为负。
                self.cash -= fee_cost;
            }
        }

        match &mut self.position {
            Some(v) => {
                if v.side == order_ref.side {
                    // 加仓
                    let old_quantity = v.quantity;
                    let new_quantity = old_quantity + order_ref.quantity;
                    let new_avg_price = (old_quantity * v.open_avg_price
                        + order_ref.quantity * order_ref.avg_price)
                        / new_quantity;

                    let fee = order_ref.avg_price * order_ref.quantity * fee_rate;

                    v.quantity = new_quantity;
                    v.open_avg_price = new_avg_price;
                    v.margin += order_ref.freeze_margin;

                    v.liquidation_price = calc_liquidation_price(
                        v.leverage,
                        self.data_source.metadata.maintenance,
                        v.side,
                        v.open_avg_price,
                        v.quantity,
                        v.margin,
                    );

                    self.pending_order_list
                        .get_mut(&v.liquidation_order_id)
                        .unwrap()
                        .price = v.liquidation_price;

                    order_ref.cumulative_quantity = order_ref.quantity;

                    v.log.push(Record {
                        id: id.to_string(),
                        kind: order_ref.kind,
                        side: order_ref.side,
                        price: order_ref.avg_price,
                        quantity: order_ref.cumulative_quantity,
                        profit: 0.0,
                        fee,
                        time: self.kline.time,
                    });
                } else {
                    // 平仓或反向开仓
                    let close_quantity = order_ref.quantity.min(v.quantity);
                    let remain_quantity = order_ref.quantity - v.quantity;
                    let close_margin = v.margin * (close_quantity / v.quantity);
                    let close_profit = if order_ref.kind == Kind::Liquidation {
                        -close_margin
                    } else if v.side == Side::Buy {
                        (order_ref.avg_price - v.open_avg_price) * close_quantity
                    } else {
                        (v.open_avg_price - order_ref.avg_price) * close_quantity
                    };

                    v.quantity -= close_quantity;
                    v.margin -= close_margin;

                    self.cash += close_margin + close_profit;

                    let close_fee = order_ref.avg_price * close_quantity * fee_rate;

                    order_ref.cumulative_quantity = order_ref.quantity; //TODO: 不要使用 close_quantity，要保证平仓和反向开仓是同一个订单

                    v.log.push(Record {
                        id: id.to_string(),
                        kind: order_ref.kind,
                        side: order_ref.side,
                        price: order_ref.avg_price,
                        quantity: order_ref.cumulative_quantity,
                        profit: close_profit,
                        fee: close_fee,
                        time: self.kline.time,
                    });

                    let profit = v.log.iter().map(|v| v.profit).sum();
                    let fee = v.log.iter().map(|v| v.fee).sum();

                    let mut max_quantity = None;
                    let mut sum: f64 = 0.0;

                    for v in v.log.iter() {
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

                    let max_quantity = max_quantity.unwrap_or(0.0);

                    let min_size = self.data_source.metadata.min_size;

                    if remain_quantity > min_size {
                        // 反向开仓
                        let reverse_quantity = remain_quantity.abs();
                        let reverse_margin = calc_initial_margin(
                            order_ref.avg_price,
                            reverse_quantity,
                            self.leverage,
                        );

                        // 下单阶段按整单冻结；进入反向后只需要为新仓保留 reverse_margin。
                        // 其余冻结保证金应返还，避免资金凭空减少。
                        self.cash += order_ref.freeze_margin - reverse_margin;

                        self.history_position_list.push(HistoryPosition {
                            symbol: v.symbol.clone(),
                            leverage: v.leverage,
                            side: v.side,
                            open_avg_price: v.open_avg_price,
                            close_avg_price: order_ref.avg_price,
                            max_quantity,
                            close_quantity: max_quantity,
                            total_profit: profit - fee,
                            profit,
                            fee,
                            open_time: v.open_time,
                            close_time: self.kline.time,
                            log: v.log.clone(),
                        });

                        let liquidation_order = self
                            .pending_order_list
                            .get_mut(&v.liquidation_order_id)
                            .unwrap();

                        liquidation_order.side = order_ref.side.neg();
                        liquidation_order.price = calc_liquidation_price(
                            self.leverage,
                            self.data_source.metadata.maintenance,
                            order_ref.side,
                            order_ref.avg_price,
                            reverse_quantity,
                            reverse_margin,
                        );

                        self.position = Some(PositionEx {
                            liquidation_order_id: liquidation_order.id.clone(),
                            log: vec![Record {
                                id: id.to_string(),
                                kind: order_ref.kind,
                                side: order_ref.side,
                                price: order_ref.avg_price,
                                quantity: reverse_quantity,
                                profit: 0.0,
                                fee: order_ref.avg_price * reverse_quantity * fee_rate,
                                time: self.kline.time,
                            }],
                            parent: Position {
                                symbol: order_ref.symbol.clone(),
                                leverage: self.leverage,
                                side: order_ref.side,
                                open_avg_price: order_ref.avg_price,
                                quantity: reverse_quantity,
                                margin: reverse_margin,
                                liquidation_price: liquidation_order.price,
                                profit: 0.0,
                                open_time: self.kline.time,
                            },
                        });
                    } else if remain_quantity.abs() <= min_size {
                        // 全部平仓
                        // 对手方向普通单若未形成新仓，整单冻结应全额返还。
                        self.cash += order_ref.freeze_margin;

                        if let Some(last_position) = self.history_position_list.iter_mut().last()
                            && !last_position.close_quantity.snap_eq(
                                last_position.max_quantity,
                                self.data_source.metadata.min_size,
                            )
                        {
                            last_position.leverage = v.leverage;
                            last_position.side = v.side;
                            last_position.open_avg_price = v.open_avg_price;
                            last_position.close_avg_price = order_ref.avg_price;
                            last_position.max_quantity = max_quantity;
                            last_position.close_quantity = max_quantity;
                            last_position.total_profit = profit - fee;
                            last_position.profit = profit;
                            last_position.fee = fee;
                            last_position.open_time = v.open_time;
                            last_position.close_time = self.kline.time;
                            last_position.log = v.log.clone();
                        } else {
                            self.history_position_list.push(HistoryPosition {
                                symbol: v.symbol.clone(),
                                leverage: v.leverage,
                                side: v.side,
                                open_avg_price: v.open_avg_price,
                                close_avg_price: order_ref.avg_price,
                                max_quantity,
                                close_quantity: max_quantity,
                                total_profit: profit - fee,
                                profit,
                                fee,
                                open_time: v.open_time,
                                close_time: self.kline.time,
                                log: v.log.clone(),
                            });
                        }

                        self.pending_order_list
                            .shift_remove(&v.liquidation_order_id);

                        self.position = None;
                    } else {
                        // 部分平仓
                        // 对手方向普通单仅执行平仓时，整单冻结应全额返还。
                        self.cash += order_ref.freeze_margin;

                        if let Some(last_position) = self.history_position_list.iter_mut().last()
                            && !last_position.close_quantity.snap_eq(
                                last_position.max_quantity,
                                self.data_source.metadata.min_size,
                            )
                        {
                            let close_quantity = v
                                .log
                                .iter()
                                .filter(|i| i.side != v.side)
                                .map(|i| i.quantity)
                                .sum::<f64>()
                                .max(0.0);

                            last_position.leverage = v.leverage;
                            last_position.side = v.side;
                            last_position.open_avg_price = v.open_avg_price;
                            last_position.max_quantity = max_quantity;
                            last_position.close_avg_price = order_ref.avg_price;
                            last_position.close_quantity = close_quantity;
                            last_position.total_profit = profit - fee;
                            last_position.profit = profit;
                            last_position.fee = fee;
                            last_position.close_time = self.kline.time;
                            last_position.log = v.log.clone();
                        } else {
                            self.history_position_list.push(HistoryPosition {
                                symbol: v.symbol.clone(),
                                leverage: v.leverage,
                                side: v.side,
                                open_avg_price: v.open_avg_price,
                                close_avg_price: order_ref.avg_price,
                                max_quantity,
                                close_quantity,
                                total_profit: profit - fee,
                                profit,
                                fee,
                                open_time: v.open_time,
                                close_time: self.kline.time,
                                log: v.log.clone(),
                            });
                        }

                        v.liquidation_price = calc_liquidation_price(
                            v.leverage,
                            self.data_source.metadata.maintenance,
                            v.side,
                            v.open_avg_price,
                            v.quantity,
                            v.margin,
                        );

                        self.pending_order_list
                            .get_mut(&v.liquidation_order_id)
                            .unwrap()
                            .price = v.liquidation_price;
                    }
                }
            }
            None => {
                // 开仓
                let liquidation_price = calc_liquidation_price(
                    self.leverage,
                    self.data_source.metadata.maintenance,
                    order_ref.side,
                    order_ref.avg_price,
                    order_ref.quantity,
                    order_ref.freeze_margin,
                );

                let liquidation_order_id = self
                    .place_order(
                        Order {
                            symbol: order_ref.symbol.clone(),
                            side: order_ref.side.neg(),
                            trigger_price: 0.0,
                            price: liquidation_price,
                            quantity: f64::MAX,
                            reduce_only: true,
                        },
                        Kind::Liquidation,
                    )
                    .unwrap();

                order_ref.cumulative_quantity = order_ref.quantity;

                self.position = Some(PositionEx {
                    liquidation_order_id,
                    log: vec![Record {
                        id: id.to_string(),
                        kind: order_ref.kind,
                        side: order_ref.side,
                        price: order_ref.avg_price,
                        quantity: order_ref.cumulative_quantity,
                        profit: 0.0,
                        fee: order_ref.avg_price * order_ref.cumulative_quantity * fee_rate,
                        time: self.kline.time,
                    }],
                    parent: Position {
                        symbol: order_ref.symbol.clone(),
                        leverage: self.leverage,
                        side: order_ref.side,
                        open_avg_price: order_ref.avg_price,
                        quantity: order_ref.cumulative_quantity,
                        margin: order_ref.freeze_margin,
                        liquidation_price,
                        profit: 0.0,
                        open_time: self.kline.time,
                    },
                });
            }
        }

        order_ref.status = Status::Filled;
        self.history_order_list
            .insert(order_ref.id.clone(), order_ref);
    }
}

#[async_trait::async_trait]
impl Exchange for LocalExchange {
    async fn next(&self, symbol: &str, _level: Level) -> anyhow::Result<Option<KLine>> {
        let mut inner = self.inner.lock().await;

        if symbol != inner.data_source.metadata.symbol {
            bail!("next: symbol mismatch: {}", symbol);
        }

        return match inner
            .data_source
            .data
            .get(inner.start_index..=inner.end_index)
            .and_then(|v| v.get(inner.index))
            .cloned()
        {
            Some(v) => {
                inner.kline = v;
                inner.index += 1;
                inner.update();
                Ok(Some(v))
            }
            None => Ok(None),
        };
    }

    async fn place_order(&self, order: Order) -> anyhow::Result<String> {
        let metadata = self
            .get_metadata(&order.symbol)
            .await
            .context(format!("place_order: {}", order.symbol))?;

        if metadata.min_size <= 0.0 || !metadata.min_size.is_finite() {
            bail!(
                "place_order: invalid metadata.min_size (must be > 0): {}",
                metadata.symbol
            );
        }

        if metadata.tick_size <= 0.0 || !metadata.tick_size.is_finite() {
            bail!(
                "place_order: invalid metadata.tick_size (must be > 0): {}",
                metadata.symbol
            );
        }

        if !order.trigger_price.is_finite()
            || !order.price.is_finite()
            || !order.quantity.is_finite()
        {
            bail!(
                "place_order: trigger_price, price and quantity must be finite: {}",
                order.symbol
            );
        }

        if order.is_limit() && order.price <= 0.0 {
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
            if order.trigger_price <= 0.0 {
                bail!(
                    "place_order: trigger price must be greater than 0: {}",
                    metadata.symbol
                );
            }

            if order.price < 0.0 {
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

            if order.price > 0.0 && !is_tick_aligned(order.price, metadata.tick_size) {
                bail!(
                    "place_order: trigger order price must align with metadata.tick_size {}: {}",
                    metadata.tick_size,
                    metadata.symbol
                );
            }
        }

        if order.quantity <= 0.0 {
            bail!(
                "place_order: quantity must be greater than 0: {}",
                metadata.symbol
            );
        }

        if !(order.reduce_only && order.quantity == f64::MAX) {
            if !is_tick_aligned(order.quantity, metadata.min_size) {
                bail!(
                    "place_order: quantity must be a multiple of metadata.min_size {}: {}",
                    metadata.min_size,
                    metadata.symbol
                );
            }

            if metadata.min_notional != 0.0
                && order.is_limit()
                && (order.price * order.quantity).snap_lt(metadata.min_notional, metadata.tick_size)
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
        order.update_time = inner.kline.time;

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
            order.update_time = inner.kline.time;

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
            .iter()
            .map(|v| v.1.to_order_message())
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
            .position
            .as_ref()
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
                trigger_price: 0.0,
                price: 0.0,
                quantity: f64::MAX,
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

        Ok(self.inner.lock().await.history_position_list.clone())
    }

    async fn append_position_margin(&self, symbol: &str, margin: f64) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("append_position_margin: {}", symbol))?;

        if margin.is_nan() || margin.is_infinite() {
            bail!(
                "append_position_margin: margin must be a finite number: {}",
                symbol,
            );
        }

        let mut inner = self.inner.lock().await;
        let cash = inner.cash;
        let maintenance = inner.data_source.metadata.maintenance;

        let (liquidation_order_id, liquidation_price, cash_delta) = match inner.position.as_mut() {
            Some(position) => {
                let new_margin = position.margin + margin;
                let init_margin =
                    position.open_avg_price * position.quantity / position.leverage as f64;

                if new_margin.snap_lt(init_margin, 1e-8) {
                    bail!(
                        "append_position_margin: {}: the initial margin of the position needs to be at least: {}",
                        symbol,
                        init_margin
                    );
                }

                if margin > 0.0 && cash.snap_lt(margin, 1e-8) {
                    bail!(
                        "append_position_margin: {}: cash shortage, adjusting margin requires additional cash: {}",
                        symbol,
                        margin,
                    );
                }

                if margin < 0.0 && margin.abs().snap_gt(position.margin, 1e-8) {
                    bail!(
                        "append_position_margin: {}: cannot reduce margin more than current margin {}",
                        symbol,
                        position.margin
                    );
                }

                position.margin = new_margin;

                position.liquidation_price = calc_liquidation_price(
                    position.leverage,
                    maintenance,
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

    async fn get_equity(&self) -> anyhow::Result<f64> {
        let inner = self.inner.lock().await;

        let pending_freeze_margin = inner
            .pending_order_list
            .values()
            .filter(|order| order.status == Status::Submitted && order.kind.is_normal())
            .map(|order| order.freeze_margin)
            .sum::<f64>();

        Ok(inner.cash
            + inner
                .position
                .as_ref()
                .map(|v| v.margin + v.profit)
                .unwrap_or_default()
            + pending_freeze_margin)
    }

    async fn get_cash(&self) -> anyhow::Result<f64> {
        Ok(self.inner.lock().await.cash)
    }

    async fn get_leverage(&self, symbol: &str) -> anyhow::Result<u32> {
        self.get_metadata(symbol)
            .await
            .context(format!("get_leverage: {}", symbol))?;

        Ok(self.inner.lock().await.leverage)
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()> {
        self.get_metadata(symbol)
            .await
            .context(format!("set_leverage: {}", symbol))?;

        if leverage == 0 {
            bail!("set_leverage: {}: leverage must be greater than 0", symbol);
        }

        let mut inner = self.inner.lock().await;
        let maintenance = inner.data_source.metadata.maintenance;

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

        let (append_margin, new_margin) = if let Some(v) = &inner.position {
            let new_margin = calc_initial_margin(v.open_avg_price, v.quantity, leverage);

            (new_margin - v.margin, new_margin)
        } else {
            (0.0, 0.0)
        };

        if append_margin > 0.0 && inner.cash.snap_lt(append_margin, 1e-8) {
            bail!(
                "set_leverage: {}: cash shortage, adjusting leverage requires additional margin: {}",
                symbol,
                append_margin
            );
        }

        inner.leverage = leverage;
        inner.cash -= append_margin;

        let liquidation_update = if let Some(v) = inner.position.as_mut() {
            v.leverage = leverage;
            v.margin = new_margin;
            v.liquidation_price = calc_liquidation_price(
                v.leverage,
                maintenance,
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

        if symbol == inner.data_source.metadata.symbol {
            Ok(inner.data_source.metadata.clone())
        } else {
            bail!("get_metadata: no symbol: {}", symbol)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMBOL: &str = "BTCUSDT";
    const SNAP_TICK: f64 = 0.00000001;

    #[track_caller]
    fn assert_snap_eq(a: f64, b: f64) {
        assert!(a.snap_eq(b, SNAP_TICK), "left: {a}, right: {b}");
    }

    fn gen_kline(time: u64, open: f64, high: f64, low: f64, close: f64) -> KLine {
        KLine {
            time,
            open,
            high,
            low,
            close,
            volume: 1.0,
        }
    }

    fn gen_kline_range() -> Vec<KLine> {
        vec![
            gen_kline(1, 100.0, 101.0, 99.0, 100.5),
            gen_kline(2, 105.0, 106.0, 104.0, 105.5),
            gen_kline(3, 110.0, 111.0, 109.0, 110.5),
        ]
    }

    fn default_metadata() -> Metadata {
        Metadata {
            symbol: SYMBOL.to_string(),
            level: Level::Minute1,
            min_size: 0.001,
            min_notional: 0.0,
            tick_size: 0.1,
            maker_fee: 0.0002,
            taker_fee: 0.0005,
            maintenance: 0.004,
        }
    }

    fn test_exchange_with(metadata: Metadata, kline_list: Vec<KLine>) -> LocalExchange {
        let data_source = DataSource::new(metadata, kline_list);
        LocalExchange::new(data_source).cash(10000.0).leverage(10)
    }

    fn test_exchange() -> LocalExchange {
        test_exchange_with(default_metadata(), gen_kline_range())
    }

    // 验证市价单在下一根 K 线按开盘价成交并生成仓位。
    #[tokio::test]
    async fn market_order_fills_on_next_kline() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 105.0);
        assert_snap_eq(position.open_avg_price, 105.0);
        assert_snap_eq(position.quantity, 1.0);
    }

    // 验证取消限价单后会返还冻结保证金，且挂单列表清空。
    #[tokio::test]
    async fn cancel_order_refunds_frozen_margin() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert!(cash_after_place.snap_lt(cash_before, SNAP_TICK));

        exchange.cancel_order(SYMBOL, &id).await.unwrap();

        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_cancel, cash_before);
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // 验证存在挂单时不允许修改杠杆，撤单后可成功修改。
    #[tokio::test]
    async fn set_leverage_fails_when_pending_orders_exist() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();

        let result = exchange.set_leverage(SYMBOL, 20).await.unwrap_err();

        assert!(result.to_string().contains("pending orders"));

        exchange.cancel_order(SYMBOL, &id).await.unwrap();
        exchange.set_leverage(SYMBOL, 20).await.unwrap();

        assert_eq!(exchange.get_leverage(SYMBOL).await.unwrap(), 20);
    }

    // 验证触发市价单立即执行：触发后在同一根 K 线立即成交。
    #[tokio::test]
    async fn trigger_market_order_is_two_stage_filled() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_id = exchange
            .buy_trigger_market(SYMBOL, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，仓位已创建
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.open_avg_price, 105.0); // 在当前K线开盘价成交
        assert_snap_eq(position.quantity, 1.0);

        // 仓位创建后会有强平单
        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, Kind::Liquidation);
    }

    // 验证无仓位时的 reduce-only 订单会被自动取消。
    #[tokio::test]
    async fn reduce_only_without_position_is_canceled() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let id = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Canceled);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证 close_all_position 会在下一根 K 线完成平仓并写入历史。
    #[tokio::test]
    async fn close_all_position_closes_on_next_kline() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_some());

        exchange.close_all_position(SYMBOL).await.unwrap();
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_some());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].close_avg_price, 110.0);
    }

    // 验证追加保证金接口的边界校验和现金/保证金更新逻辑。
    #[tokio::test]
    async fn append_position_margin_checks_and_updates_cash() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let result = exchange
            .append_position_margin(SYMBOL, -1.0)
            .await
            .unwrap_err();

        assert!(result.to_string().contains("initial margin"));

        let cash_before = exchange.get_cash().await.unwrap();
        let margin_before = exchange.get_position(SYMBOL).await.unwrap().unwrap().margin;
        let liquidation_price_before = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(SYMBOL, 2.0).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let margin_after = position_after.margin;
        let liquidation_price_after = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert_snap_eq(cash_before - cash_after, 2.0);
        assert_snap_eq(margin_after - margin_before, 2.0);
        assert_snap_eq(liquidation_price_after, position_after.liquidation_price);
        assert!(liquidation_price_after.snap_lt(liquidation_price_before, SNAP_TICK));
    }

    // 验证权益会随未实现盈亏在后续 K 线上动态变化。
    #[tokio::test]
    async fn equity_tracks_unrealized_profit_over_klines() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let equity_after_open = exchange.get_equity().await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let equity_after_next = exchange.get_equity().await.unwrap();

        assert!(equity_after_next > equity_after_open);
        assert_snap_eq(equity_after_next - equity_after_open, 5.0);
    }

    // 验证空仓（Sell）未实现盈亏方向正确：价格下跌时权益上升。
    #[tokio::test]
    async fn equity_tracks_unrealized_profit_for_short_position() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 100.0, 101.0, 99.0, 100.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let equity_after_open = exchange.get_equity().await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let equity_after_next = exchange.get_equity().await.unwrap();

        assert!(equity_after_next > equity_after_open);
        assert_snap_eq(equity_after_next - equity_after_open, 5.0);
    }

    // 验证限价单价格在当根 K 线区间内时能够成交。
    #[tokio::test]
    async fn limit_order_filled_when_price_in_range() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 105.0, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 106.0);
        assert_snap_eq(position.open_avg_price, 106.0);
    }

    // 验证限价单价格不在区间内时保持 Submitted 不成交。
    #[tokio::test]
    async fn limit_order_not_filled_when_price_out_of_range() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证市价单成交价使用开盘价而非高低价。
    #[tokio::test]
    async fn market_order_uses_open_price_not_high_low() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_snap_eq(order.avg_price, 105.0);
        assert!(order.avg_price.snap_lt(106.0, SNAP_TICK));
        assert!(order.avg_price.snap_gt(104.0, SNAP_TICK));
    }

    // 验证卖出 reduce-only 能正确平掉已有多仓。
    #[tokio::test]
    async fn sell_market_order_fills_correctly() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_id = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();

        assert_eq!(close_order.status, Status::Filled);
        assert_snap_eq(close_order.avg_price, 110.0);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
    }

    // 验证 reduce-only 平仓在手续费预扣不足时会被拒绝，且不会改变现金和仓位。
    #[tokio::test]
    async fn reduce_only_close_rejected_when_fee_precharge_cash_is_insufficient() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(10.56)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();
        let position_before_close = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let close_id = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(close_order.status, Status::Rejected);
        assert_snap_eq(cash_after, cash_before_close);
        assert_eq!(position_after.side, position_before_close.side);
        assert_snap_eq(position_after.quantity, position_before_close.quantity);
        assert_snap_eq(
            position_after.open_avg_price,
            position_before_close.open_avg_price,
        );
    }

    // 验证浮亏场景下，手续费不足的 reduce-only 平仓会被拒绝，不会把现金打到负值。
    #[tokio::test]
    async fn reduce_only_close_with_floating_loss_and_fee_shortage_is_rejected() {
        let exchange = LocalExchange::new(DataSource::new(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 80.0, 81.0, 79.0, 80.0),
            ],
        ))
        .cash(10.56)
        .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();

        let close_id = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(close_order.status, Status::Rejected);
        assert_eq!(position_after.side, Side::Buy);
        assert_snap_eq(position_after.quantity, 1.0);
        assert_snap_eq(cash_after, cash_before_close);
        assert!(cash_after.snap_gt(0.0, SNAP_TICK));
    }

    // 验证触发限价单触发后立即在同一根 K 线成交。
    #[tokio::test]
    async fn trigger_limit_order_triggers_then_fills_on_next_kline() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 102.0, 106.0, 101.0, 105.0),
                gen_kline(3, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let trigger_id = exchange
            .buy_trigger_limit(SYMBOL, 104.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，仓位已创建
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.open_avg_price, 106.0); // 买单限价≥开盘价，以最高价成交
    }

    // 验证触发价长期不满足时，触发单保持 Submitted 状态。
    #[tokio::test]
    async fn trigger_order_stays_submitted_when_not_triggered() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_market(SYMBOL, 200.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证同向加仓后仓位数量累加且均价按加权方式更新。
    #[tokio::test]
    async fn add_position_updates_avg_price_correctly() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.quantity, 2.0);
        assert_snap_eq(position.open_avg_price, 107.5);
    }

    // 验证反向超量下单会先平仓再反向开仓。
    #[tokio::test]
    async fn reverse_position_long_to_short() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_snap_eq(position.quantity, 1.0);
        assert_snap_eq(position.open_avg_price, 110.0);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
    }

    // 验证反向剩余量小于 min_size 时不会误开微小反向仓。
    #[tokio::test]
    async fn opposite_order_with_tiny_positive_remainder_closes_without_reversal() {
        let mut metadata = default_metadata();
        metadata.min_size = 0.001;

        let exchange = test_exchange_with(
            metadata,
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 比原仓位多 0.0005，小于 min_size=0.001，应按全平处理，不反向开仓。
        exchange.sell(SYMBOL, 1.0005).await.unwrap_err();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_some());
    }

    // 验证开仓后会自动生成对应的强平保护订单。
    #[tokio::test]
    async fn liquidation_order_created_on_open() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert!(pending.iter().any(|v| v.kind == Kind::Liquidation));
    }

    // 验证普通单会冻结保证金，而 reduce-only 订单不会冻结保证金。
    #[tokio::test]
    async fn freeze_margin_only_for_non_reduce_only_orders() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let normal_id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let cash_after_normal = exchange.get_cash().await.unwrap();

        let reduce_only_id = exchange
            .sell_limit_reduce_only(SYMBOL, 120.0, 1.0)
            .await
            .unwrap();
        let cash_after_reduce_only = exchange.get_cash().await.unwrap();

        assert!(cash_after_normal.snap_lt(cash_before, SNAP_TICK));
        assert_snap_eq(cash_after_reduce_only, cash_after_normal);

        exchange.cancel_order(SYMBOL, &normal_id).await.unwrap();
        exchange
            .cancel_order(SYMBOL, &reduce_only_id)
            .await
            .unwrap();
    }

    // 验证取消不存在的订单是幂等操作，不会报错且不污染状态。
    #[tokio::test]
    async fn cancel_order_nonexistent_is_idempotent() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        exchange.cancel_order(SYMBOL, "not-exists").await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert_snap_eq(cash_after, cash_before);
        assert!(pending.is_empty());
    }

    // 验证下单参数含非有限值（NaN/Infinity）时会被拒绝。
    #[tokio::test]
    async fn place_order_rejects_non_finite_values() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let nan_err = exchange
            .place_order(Order {
                symbol: SYMBOL.to_string(),
                side: Side::Buy,
                trigger_price: 0.0,
                price: f64::NAN,
                quantity: 1.0,
                reduce_only: false,
            })
            .await
            .unwrap_err();

        let inf_err = exchange
            .place_order(Order {
                symbol: SYMBOL.to_string(),
                side: Side::Buy,
                trigger_price: 0.0,
                price: f64::INFINITY,
                quantity: 1.0,
                reduce_only: false,
            })
            .await
            .unwrap_err();

        assert!(nan_err.to_string().contains("must be finite"));
        assert!(inf_err.to_string().contains("must be finite"));
    }

    // 验证限价单价格不能为负，避免污染保证金与现金计算。
    #[tokio::test]
    async fn place_order_rejects_negative_limit_price() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let negative_err = exchange.buy_limit(SYMBOL, -1.0, 1.0).await.unwrap_err();

        assert!(
            negative_err
                .to_string()
                .contains("limit price must be greater than 0")
        );
    }

    // 验证触发单触发价必须大于 0，且触发限价单价格不能为负。
    #[tokio::test]
    async fn place_order_rejects_invalid_trigger_prices() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_price_err = exchange
            .place_order(Order {
                symbol: SYMBOL.to_string(),
                side: Side::Buy,
                trigger_price: -1.0,
                price: 0.0,
                quantity: 1.0,
                reduce_only: false,
            })
            .await
            .unwrap_err();

        let trigger_limit_negative_price_err = exchange
            .buy_trigger_limit(SYMBOL, 100.0, -1.0, 1.0)
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
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange
            .buy_limit(SYMBOL, 68000.123, 1.0)
            .await
            .unwrap_err();
    }

    // 验证通过浮点运算得到的档位价格（如 0.1 + 0.2）不会被误判为非对齐。
    #[tokio::test]
    async fn place_order_accepts_limit_price_from_float_arithmetic_when_tick_aligned() {
        let mut metadata = default_metadata();
        metadata.tick_size = 0.1;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let computed_price = 0.1_f64 + 0.2_f64;
        let id = exchange
            .buy_limit(SYMBOL, computed_price, 1.0)
            .await
            .unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证由浮点算术构造的价格/数量在复杂流程中保持稳定：
    // 限价开仓 -> 触发限价减仓 -> 普通限价挂单并撤单，且每一步都断言现金/权益/仓位/挂单/历史。
    #[tokio::test]
    async fn float_arithmetic_complex_step_assertions_on_each_transition() {
        let mut metadata = default_metadata();
        metadata.tick_size = 0.1;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(
            metadata.clone(),
            vec![
                gen_kline(1, 100.0, 100.5, 99.8, 100.1),
                gen_kline(2, 100.2, 100.6, 99.8, 100.4),
                gen_kline(3, 100.5, 100.9, 100.3, 100.7),
                gen_kline(4, 100.6, 100.8, 100.1, 100.2),
            ],
        );

        let initial_cash = 10000.0;

        let entry_price = 100.0 + (0.1_f64 + 0.2_f64);
        let entry_qty = 0.1_f64 + 0.2_f64;

        let trigger_price = 100.0 + (0.2_f64 + 0.3_f64);
        let reduce_limit_price = 100.0 + (0.2_f64 + 0.2_f64);
        let reduce_qty = 0.1_f64 + 0.1_f64;

        let md = metadata;

        // Step 1: 仅推进首根 K，状态不变。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), initial_cash);
        assert_snap_eq(exchange.get_equity().await.unwrap(), initial_cash);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );

        // 下由浮点算术构造的限价开多。
        let open_id = exchange
            .buy_limit(SYMBOL, entry_price, entry_qty)
            .await
            .unwrap();

        let freeze_entry = entry_price * entry_qty / 10.0;
        let cash_after_place = initial_cash - freeze_entry;

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_place);
        // 新权益口径包含挂单冻结保证金，因此总权益不变。
        assert_snap_eq(exchange.get_equity().await.unwrap(), initial_cash);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );

        // Step 2: 限价开仓成交（buy 且 price>=open，按 high 成交）。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&open_id).await.unwrap().unwrap();
        let mut position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let pending_s2 = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        let open_fill = 100.6;
        let margin_s2 = open_fill * entry_qty / 10.0;
        let open_fee = open_fill * entry_qty * md.maker_fee;
        let cash_s2 = initial_cash - margin_s2 - open_fee;
        let upnl_s2 = (100.4 - open_fill) * entry_qty;
        let equity_s2 = cash_s2 + margin_s2 + upnl_s2;

        assert_eq!(open_order.status, Status::Filled);
        assert_snap_eq(open_order.avg_price, open_fill);
        assert_snap_eq(position.open_avg_price, open_fill);
        assert_snap_eq(position.quantity, entry_qty);
        assert_snap_eq(position.margin, margin_s2);
        assert_snap_eq(position.profit, upnl_s2);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(pending_s2.len(), 1);
        assert_eq!(pending_s2[0].kind, Kind::Liquidation);

        // 下由浮点算术构造的 reduce-only 触发限价平仓单。
        let trigger_reduce_id = exchange
            .sell_trigger_limit_reduce_only(SYMBOL, trigger_price, reduce_limit_price, reduce_qty)
            .await
            .unwrap();

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // Step 3: 触发单触发并立即执行（sell 且 price<=open，按 low 成交）。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_reduce = exchange
            .get_order(&trigger_reduce_id)
            .await
            .unwrap()
            .unwrap();
        let pending_s3 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let reduce_limit_fill = 100.3; // kline 3 low
        let close_margin = margin_s2 * (reduce_qty / entry_qty);
        let close_profit = (reduce_limit_fill - open_fill) * reduce_qty;
        let close_fee = reduce_limit_fill * reduce_qty * md.maker_fee;
        let cash_s3 = cash_s2 - close_fee + close_margin + close_profit;
        let qty_left = entry_qty - reduce_qty;
        let margin_left = margin_s2 - close_margin;
        let upnl_s3 = (100.7 - open_fill) * qty_left;
        let equity_s3 = cash_s3 + margin_left + upnl_s3;

        assert_eq!(trigger_reduce.status, Status::Filled);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s3);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s3);
        assert_snap_eq(position.quantity, qty_left);
        assert_snap_eq(position.margin, margin_left);
        assert_snap_eq(position.profit, upnl_s3);
        assert_eq!(pending_s3.len(), 1);
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Liquidation));

        // Step 4: 无新订单成交，仅推进 K 线。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let pending_s4 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let history_pos_s4 = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let position_s4 = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let upnl_s4 = (100.2 - open_fill) * qty_left;
        let equity_s4 = cash_s3 + margin_left + upnl_s4;

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s3);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(position_s4.side, Side::Buy);
        assert_snap_eq(position_s4.open_avg_price, open_fill);
        assert_snap_eq(position_s4.quantity, qty_left);
        assert_snap_eq(position_s4.margin, margin_left);
        assert_snap_eq(position_s4.profit, upnl_s4);
        assert_eq!(pending_s4.len(), 1);
        assert_eq!(pending_s4[0].kind, Kind::Liquidation);
        assert_eq!(history_pos_s4.len(), 1);
        assert_snap_eq(history_pos_s4[0].close_quantity, reduce_qty);
        assert_snap_eq(history_pos_s4[0].close_avg_price, reduce_limit_fill);

        // 再挂一个浮点算术价格限价单，验证冻结保证金已计入权益，然后立即撤单。
        let readd_price = 99.7_f64 + 0.3_f64;
        let readd_qty = 0.05_f64 + 0.05_f64;
        let readd_id = exchange
            .buy_limit(SYMBOL, readd_price, readd_qty)
            .await
            .unwrap();

        let readd_freeze = readd_price * readd_qty / 10.0;
        let cash_after_readd_place = cash_s3 - readd_freeze;

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_readd_place);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        exchange.cancel_order(SYMBOL, &readd_id).await.unwrap();

        let canceled = exchange.get_order(&readd_id).await.unwrap().unwrap();
        assert_eq!(canceled.status, Status::Canceled);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s3);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );
    }

    // 验证多个浮点算术限价单在“部分成交+保留挂单+撤单+平仓”路径中的逐步状态一致性。
    #[tokio::test]
    async fn float_arithmetic_multi_order_step_assertions_with_cancel_and_close() {
        let mut metadata = default_metadata();
        metadata.tick_size = 0.1;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(
            metadata.clone(),
            vec![
                gen_kline(1, 100.0, 100.4, 99.8, 100.0),
                gen_kline(2, 100.2, 100.5, 100.0, 100.3),
                gen_kline(3, 100.4, 100.6, 100.1, 100.2),
            ],
        );

        let md = metadata;
        let initial_cash = 10000.0;

        let fill_price_order = 100.0 + (0.2_f64 + 0.1_f64);
        let fill_qty_order = 0.15_f64 + 0.15_f64;

        let pending_price_order = 99.6_f64 + (0.1_f64 + 0.1_f64);
        let pending_qty_order = 0.05_f64 + 0.05_f64;

        // Step 1: 首根 K，状态不变。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), initial_cash);
        assert_snap_eq(exchange.get_equity().await.unwrap(), initial_cash);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());

        // 同时挂两个由浮点算术构造的买限价：一个预期成交、一个预期保留。
        let id_fill = exchange
            .buy_limit(SYMBOL, fill_price_order, fill_qty_order)
            .await
            .unwrap();
        let id_pending = exchange
            .buy_limit(SYMBOL, pending_price_order, pending_qty_order)
            .await
            .unwrap();

        let freeze_fill = fill_price_order * fill_qty_order / 10.0;
        let freeze_pending = pending_price_order * pending_qty_order / 10.0;
        let cash_after_place = initial_cash - freeze_fill - freeze_pending;

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_place);
        assert_snap_eq(exchange.get_equity().await.unwrap(), initial_cash);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // Step 2: 第二根 K，仅第一笔成交（按 high 成交），第二笔保留为 pending。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order_fill = exchange.get_order(&id_fill).await.unwrap().unwrap();
        let order_pending = exchange.get_order(&id_pending).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let fill_avg = 100.5;
        let margin_s2 = fill_avg * fill_qty_order / 10.0;
        let fee_open = fill_avg * fill_qty_order * md.maker_fee;
        let cash_s2 = initial_cash - freeze_pending - margin_s2 - fee_open;
        let upnl_s2 = (100.3 - fill_avg) * fill_qty_order;
        let equity_s2 = cash_s2 + margin_s2 + upnl_s2 + freeze_pending;

        assert_eq!(order_fill.status, Status::Filled);
        assert_eq!(order_pending.status, Status::Submitted);
        assert_snap_eq(order_fill.avg_price, fill_avg);

        assert_eq!(position.side, Side::Buy);
        assert_snap_eq(position.open_avg_price, fill_avg);
        assert_snap_eq(position.quantity, fill_qty_order);
        assert_snap_eq(position.margin, margin_s2);
        assert_snap_eq(position.profit, upnl_s2);

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // 撤掉仍 pending 的普通限价单，现金回补，但总权益应保持不变。
        exchange.cancel_order(SYMBOL, &id_pending).await.unwrap();

        let canceled_pending = exchange.get_order(&id_pending).await.unwrap().unwrap();
        let cash_after_cancel = cash_s2 + freeze_pending;
        let equity_after_cancel = cash_after_cancel + margin_s2 + upnl_s2;

        assert_eq!(canceled_pending.status, Status::Canceled);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_cancel);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_after_cancel);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );

        // 下由浮点算术构造的 reduce-only 市价平仓单。
        let close_qty = 0.1_f64 + 0.2_f64;
        let close_id = exchange.sell_reduce_only(SYMBOL, close_qty).await.unwrap();

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_cancel);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // Step 3: 第三根 K 市价平仓成交。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let pending_s3 = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        let close_avg = 100.4;
        let close_profit = (close_avg - fill_avg) * close_qty;
        let close_fee = close_avg * close_qty * md.taker_fee;
        let cash_final = cash_after_cancel - close_fee + margin_s2 + close_profit;

        assert_eq!(close_order.status, Status::Filled);
        assert_snap_eq(close_order.avg_price, close_avg);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(pending_s3.is_empty());

        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].open_avg_price, fill_avg);
        assert_snap_eq(history[0].close_avg_price, close_avg);
        assert_snap_eq(history[0].close_quantity, close_qty);

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_final);
        assert_snap_eq(exchange.get_equity().await.unwrap(), cash_final);
    }

    // 验证触发限价单触发价与委托价都必须与 tick_size 对齐。
    #[tokio::test]
    async fn place_order_rejects_trigger_prices_not_aligned_to_tick_size() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange
            .buy_trigger_market(SYMBOL, 105.05, 1.0)
            .await
            .unwrap_err();
        exchange
            .buy_trigger_limit(SYMBOL, 105.0, 105.05, 1.0)
            .await
            .unwrap_err();
    }

    // 验证名义价值低于最小限制时会拒绝下单。
    #[tokio::test]
    async fn place_order_rejects_below_min_notional() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let result = exchange.buy_limit(SYMBOL, 10.0, 5.0).await.unwrap_err();

        assert!(result.to_string().contains("metadata.min_notional"));
    }

    // 验证下单数量低于最小下单量时会拒绝下单。
    #[tokio::test]
    async fn place_order_rejects_below_min_size() {
        let mut metadata = default_metadata();
        metadata.min_size = 0.01;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy_limit(SYMBOL, 100.0, 0.001).await.unwrap_err();
    }

    // 验证 min_size 校验不会因 round 导致“略小于最小值”被错误放行。
    #[tokio::test]
    async fn place_order_rejects_quantity_slightly_below_min_size() {
        let mut metadata = default_metadata();
        metadata.min_size = 0.01;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy_limit(SYMBOL, 100.0, 0.0096).await.unwrap_err();
    }

    // 验证 cancel_all_order 仅取消普通 Submitted 订单，不影响强平保护单。
    #[tokio::test]
    async fn cancel_all_order_only_cancels_submitted_normal_orders() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let pending_before = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        assert_eq!(pending_before.len(), 2);
        assert!(pending_before.iter().any(|v| v.kind == Kind::Limit));
        assert!(pending_before.iter().any(|v| v.kind == Kind::Liquidation));

        exchange.cancel_all_order(SYMBOL).await.unwrap();

        let pending_after = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let history = exchange.get_history_order_list(SYMBOL).await.unwrap();

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
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(SYMBOL, 200.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.kind, Kind::Trigger);
        assert_eq!(order.status, Status::Submitted);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证限价单穿价按最差价成交时，现金差值符合“价格差+费率差”的公式。
    #[tokio::test]
    async fn maker_vs_taker_fee_difference_on_same_entry_price() {
        let market_exchange = test_exchange();
        market_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        market_exchange.buy(SYMBOL, 1.0).await.unwrap();
        market_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let market_cash = market_exchange.get_cash().await.unwrap();

        let limit_exchange = test_exchange();
        limit_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        limit_exchange.buy_limit(SYMBOL, 105.0, 1.0).await.unwrap();
        limit_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let limit_cash = limit_exchange.get_cash().await.unwrap();

        assert!(limit_cash.snap_lt(market_cash, SNAP_TICK));

        let market_cost = 105.0 / 10.0 + 105.0 * default_metadata().taker_fee;
        let limit_cost = 106.0 / 10.0 + 106.0 * default_metadata().maker_fee;
        assert_snap_eq(limit_cash - market_cash, market_cost - limit_cost);
    }

    // 验证调低杠杆倍率（更高保证金要求）时会重算仓位保证金与可用现金。
    #[tokio::test]
    async fn set_leverage_recalculates_position_margin_and_cash() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liquidation_price_before = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.set_leverage(SYMBOL, 5).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liquidation_price_after = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert_eq!(position_after.leverage, 5);
        assert_snap_eq(position_before.margin, 10.5);
        assert_snap_eq(position_after.margin, 21.0);
        assert_snap_eq(cash_before - cash_after, 10.5);
        assert_snap_eq(liquidation_price_after, position_after.liquidation_price);
        assert!(liquidation_price_after.snap_lt(liquidation_price_before, SNAP_TICK));
    }

    // 验证调低杠杆导致需要补充保证金但现金不足时会失败。
    #[tokio::test]
    async fn set_leverage_fails_when_cash_insufficient_for_new_margin() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(11.0)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let result = exchange.set_leverage(SYMBOL, 1).await.unwrap_err();

        assert!(result.to_string().contains("requires additional margin"));
        assert_eq!(exchange.get_leverage(SYMBOL).await.unwrap(), 10);
    }

    // 验证触发限价单在触发瞬间若无法冻结保证金会被拒绝。
    #[tokio::test]
    async fn trigger_limit_order_rejected_when_freeze_margin_fails() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(1.0)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(SYMBOL, 105.0, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert!(pending.is_empty());
    }

    // 验证部分平仓后仍保留剩余仓位，并记录历史平仓数量。
    #[tokio::test]
    async fn partial_close_keeps_remaining_position_and_updates_history() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 120.0, 121.0, 119.0, 120.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();

        assert_snap_eq(position.quantity, 1.0);
        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].close_quantity, 1.0);
        assert_snap_eq(history[0].close_avg_price, 120.0);
    }

    // 验证反向开仓后，强平保护单方向与价格会同步到新仓位。
    #[tokio::test]
    async fn reverse_position_updates_liquidation_order_side_and_price() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let liquidation = pending
            .iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(liquidation.side, Side::Buy);
        assert_snap_eq(liquidation.price, position.liquidation_price);
    }

    // 验证追加保证金后强平价下降、再减少保证金后强平价回升（多仓场景）。
    #[tokio::test]
    async fn append_and_reduce_margin_moves_liquidation_price_monotonic() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_before = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(SYMBOL, 2.0).await.unwrap();
        let liq_after_add = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        exchange.append_position_margin(SYMBOL, -1.0).await.unwrap();
        let liq_after_reduce = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.price)
            .unwrap();

        assert!(liq_after_add.snap_lt(liq_before, SNAP_TICK));
        assert!(liq_after_reduce.snap_gt(liq_after_add, SNAP_TICK));
    }

    // 验证仅存在强平保护单时允许调杠杆，且挂单数量不变化。
    #[tokio::test]
    async fn set_leverage_allowed_with_only_liquidation_order() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let pending_before = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        assert_eq!(pending_before.len(), 1);
        assert_eq!(pending_before[0].kind, Kind::Liquidation);

        exchange.set_leverage(SYMBOL, 5).await.unwrap();

        let pending_after = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(exchange.get_leverage(SYMBOL).await.unwrap(), 5);
        assert_eq!(pending_after.len(), 1);
        assert_eq!(pending_after[0].kind, Kind::Liquidation);
        assert_snap_eq(pending_after[0].price, position_after.liquidation_price);
    }

    // 验证调杠杆失败时，仓位保证金与强平价格不会被污染。
    #[tokio::test]
    async fn set_leverage_failure_keeps_position_and_liquidation_unchanged() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(11.0)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liquidation_before = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        let _ = exchange.set_leverage(SYMBOL, 1).await.unwrap_err();

        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liquidation_after = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(exchange.get_leverage(SYMBOL).await.unwrap(), 10);
        assert_snap_eq(position_after.margin, position_before.margin);
        assert_snap_eq(
            position_after.liquidation_price,
            position_before.liquidation_price,
        );
        assert_snap_eq(liquidation_after.price, liquidation_before.price);
    }

    // 验证升杠杆（保证金需求降低）会返还现金，并抬升多仓强平价。
    #[tokio::test]
    async fn set_leverage_higher_reduces_margin_and_returns_cash() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let position_before = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        exchange.set_leverage(SYMBOL, 20).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(position_after.leverage, 20);
        assert!(
            position_after
                .margin
                .snap_lt(position_before.margin, SNAP_TICK)
        );
        assert!(cash_after.snap_gt(cash_before, SNAP_TICK));
        assert!(
            position_after
                .liquidation_price
                .snap_gt(position_before.liquidation_price, SNAP_TICK)
        );
    }

    // 验证无仓位时调杠杆只更新全局杠杆，不影响现金与挂单。
    #[tokio::test]
    async fn set_leverage_without_position_only_updates_config() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let pending_before = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        exchange.set_leverage(SYMBOL, 25).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let pending_after = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert_eq!(exchange.get_leverage(SYMBOL).await.unwrap(), 25);
        assert_snap_eq(cash_after, cash_before);
        assert_eq!(pending_after.len(), pending_before.len());
    }

    // 验证无仓位时追加保证金会失败，且现金不会发生变化。
    #[tokio::test]
    async fn append_position_margin_without_position_keeps_cash_unchanged() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let result = exchange
            .append_position_margin(SYMBOL, 1.0)
            .await
            .unwrap_err();
        let cash_after = exchange.get_cash().await.unwrap();

        assert!(result.to_string().contains("no position"));
        assert_snap_eq(cash_after, cash_before);
    }

    // 验证无仓位时 close_all_position 是幂等空操作，不创建订单。
    #[tokio::test]
    async fn close_all_position_without_position_is_noop() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.close_all_position(SYMBOL).await.unwrap();

        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let history_order = exchange.get_history_order_list(SYMBOL).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(pending.is_empty());
        assert!(history_order.is_empty());
    }

    // 验证存在普通限价挂单时调杠杆失败，且杠杆与现金保持不变。
    #[tokio::test]
    async fn set_leverage_with_limit_pending_keeps_state_unchanged() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let leverage_before = exchange.get_leverage(SYMBOL).await.unwrap();
        let cash_before = exchange.get_cash().await.unwrap();
        let _id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();

        let result = exchange.set_leverage(SYMBOL, 15).await.unwrap_err();

        let leverage_after = exchange.get_leverage(SYMBOL).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(leverage_after, leverage_before);
        assert_snap_eq(cash_after, cash_before - 9.0);
    }

    // 验证存在普通触发挂单时调杠杆失败，且不会污染仓位状态。
    #[tokio::test]
    async fn set_leverage_with_trigger_pending_keeps_position_unchanged() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let _trigger_id = exchange
            .buy_trigger_market(SYMBOL, 200.0, 1.0)
            .await
            .unwrap();

        let result = exchange.set_leverage(SYMBOL, 8).await.unwrap_err();

        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(position_after.leverage, position_before.leverage);
        assert_snap_eq(position_after.margin, position_before.margin);
        assert_snap_eq(
            position_after.liquidation_price,
            position_before.liquidation_price,
        );
    }

    // 验证强平订单被触发后会平掉仓位，并在历史仓位中标记为强平。
    #[tokio::test]
    async fn liquidation_order_closes_position_and_records_liquidation_history() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 95.0, 96.0, 90.0, 92.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_order = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();
        assert!(liq_order.price.snap_ge(90.0, SNAP_TICK));
        assert!(liq_order.price.snap_le(96.0, SNAP_TICK));

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
    }

    // 验证 1x 多仓强平价受 maintenance 约束，并在触及阈值时执行强平。
    #[tokio::test]
    async fn one_x_long_liquidation_price_respects_maintenance() {
        let exchange = LocalExchange::new(DataSource::new(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 100.0, 101.0, 99.0, 100.0),
                gen_kline(3, 60.0, 61.0, 1.0, 10.0),
                gen_kline(4, 1.0, 1.0, 0.3, 0.5),
            ],
        ))
        .cash(10000.0)
        .leverage(1);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_after_open = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_after_open = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position_after_open.leverage, 1);
        assert_snap_eq(position_after_open.liquidation_price, 0.4);
        assert_snap_eq(liq_after_open.price, 0.4);
        assert_eq!(liq_after_open.side, Side::Sell);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_some());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_snap_eq(history[0].close_avg_price, 0.4);
    }

    // 验证 1x 空仓强平价受 maintenance 约束，并在触及阈值时执行强平。
    #[tokio::test]
    async fn one_x_short_liquidation_price_respects_maintenance() {
        let exchange = LocalExchange::new(DataSource::new(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 100.0, 101.0, 99.0, 100.0),
                gen_kline(3, 180.0, 181.0, 179.0, 180.0),
                gen_kline(4, 200.0, 200.0, 199.0, 199.8),
            ],
        ))
        .cash(10000.0)
        .leverage(1);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_after_open = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_after_open = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position_after_open.leverage, 1);
        assert_snap_eq(position_after_open.liquidation_price, 199.6);
        assert_snap_eq(liq_after_open.price, 199.6);
        assert_eq!(liq_after_open.side, Side::Buy);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_some());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_snap_eq(history[0].close_avg_price, 199.6);
    }

    // 验证存在待成交市价单时调杠杆会失败，并保持杠杆值不变。
    #[tokio::test]
    async fn set_leverage_with_market_pending_keeps_leverage_unchanged() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let leverage_before = exchange.get_leverage(SYMBOL).await.unwrap();
        let _id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        let result = exchange.set_leverage(SYMBOL, 30).await.unwrap_err();

        assert!(result.to_string().contains("pending orders"));
        assert_eq!(
            exchange.get_leverage(SYMBOL).await.unwrap(),
            leverage_before
        );
    }

    // 验证 cancel_all 同时取消 trigger/limit 时，现金仅返还限价单冻结保证金。
    #[tokio::test]
    async fn cancel_all_refunds_only_frozen_margin_orders() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let cash_after_limit = exchange.get_cash().await.unwrap();

        exchange
            .buy_trigger_limit(SYMBOL, 200.0, 90.0, 1.0)
            .await
            .unwrap();
        let cash_after_trigger = exchange.get_cash().await.unwrap();

        assert!(cash_after_limit.snap_lt(cash_before, SNAP_TICK));
        assert_snap_eq(cash_after_trigger, cash_after_limit);

        exchange.cancel_all_order(SYMBOL).await.unwrap();

        let cash_after_cancel_all = exchange.get_cash().await.unwrap();
        assert_snap_eq(cash_after_cancel_all, cash_before);
    }

    // 验证减少保证金到“初始保证金边界值”是允许的。
    #[tokio::test]
    async fn append_position_margin_allows_reducing_to_initial_margin_boundary() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.append_position_margin(SYMBOL, 2.0).await.unwrap();

        let position_after_add = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let init_margin = position_after_add.open_avg_price * position_after_add.quantity
            / position_after_add.leverage as f64;
        let reduce_to_boundary = init_margin - position_after_add.margin;

        exchange
            .append_position_margin(SYMBOL, reduce_to_boundary)
            .await
            .unwrap();

        let position_after_reduce = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_reduce.margin, init_margin);
    }

    // 验证限价成交在浮点边界值上仍保持预期精度（价格=K线高点）。
    #[tokio::test]
    async fn limit_order_fill_on_exact_high_boundary_precision() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_limit(SYMBOL, 106.0 - 1e-12, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 106.0 - 1e-12);
    }

    // 验证浮点微小超边界时限价不应成交（避免误成交）。
    #[tokio::test]
    async fn limit_order_not_fill_when_slightly_above_high() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_limit(SYMBOL, 104.0 - 1e-12, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证买入限价在价格等于 K 线 low 边界时可以成交。
    #[tokio::test]
    async fn buy_limit_fills_when_price_equals_low_boundary() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 104.0, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 104.0);
        assert_eq!(position.side, Side::Buy);
        assert_snap_eq(position.open_avg_price, 104.0);
    }

    // 验证卖出限价在价格等于 K 线 high 边界时可以成交。
    #[tokio::test]
    async fn sell_limit_fills_when_price_equals_high_boundary() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.sell_limit(SYMBOL, 106.0, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 106.0);
        assert_eq!(position.side, Side::Sell);
        assert_snap_eq(position.open_avg_price, 106.0);
    }

    // 验证买入限价略低于 low 边界时不会成交。
    #[tokio::test]
    async fn buy_limit_not_fill_when_slightly_below_low_boundary() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_limit(SYMBOL, 104.0 - 1e-12, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证卖出限价略高于 high 边界时不会成交。
    #[tokio::test]
    async fn sell_limit_not_fill_when_slightly_above_high_boundary() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .sell_limit(SYMBOL, 106.0 + 1e-12, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Submitted);
    }

    // 验证多次追加/减少保证金后，现金回到原值（控制浮点累计误差）。
    #[tokio::test]
    async fn append_margin_round_trip_keeps_cash_precision() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();

        exchange.append_position_margin(SYMBOL, 0.1).await.unwrap();
        exchange.append_position_margin(SYMBOL, -0.1).await.unwrap();
        exchange.append_position_margin(SYMBOL, 0.2).await.unwrap();
        exchange.append_position_margin(SYMBOL, -0.2).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after, cash_before);
    }

    // 验证小数数量下限价穿价成交时现金差值仍保持数学一致性。
    #[tokio::test]
    async fn maker_taker_fee_delta_with_fractional_quantity_precision() {
        let qty = 0.123;

        let market_exchange = test_exchange();
        market_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        market_exchange.buy(SYMBOL, qty).await.unwrap();
        market_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let market_cash = market_exchange.get_cash().await.unwrap();

        let limit_exchange = test_exchange();
        limit_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        limit_exchange.buy_limit(SYMBOL, 105.0, qty).await.unwrap();
        limit_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let limit_cash = limit_exchange.get_cash().await.unwrap();

        let market_cost = 105.0 * qty / 10.0 + 105.0 * qty * default_metadata().taker_fee;
        let limit_cost = 106.0 * qty / 10.0 + 106.0 * qty * default_metadata().maker_fee;

        assert!(limit_cash.snap_lt(market_cash, SNAP_TICK));
        assert_snap_eq(limit_cash - market_cash, market_cost - limit_cost);
    }

    // 验证分数数量市价开仓时，现金扣减严格符合“保证金+taker手续费”公式。
    #[tokio::test]
    async fn market_open_fractional_quantity_cash_matches_formula() {
        let qty = 0.333;
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let expected_margin = 105.0 * qty / 10.0;
        let expected_fee = 105.0 * qty * default_metadata().taker_fee;
        let expected_cash = 10000.0 - expected_margin - expected_fee;

        assert_snap_eq(cash_after, expected_cash);
    }

    // 验证分数数量限价开仓时，现金扣减严格符合“保证金+maker手续费”公式。
    #[tokio::test]
    async fn limit_open_fractional_quantity_cash_matches_formula() {
        let qty = 0.333;
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy_limit(SYMBOL, 105.0, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let expected_margin = 106.0 * qty / 10.0;
        let expected_fee = 106.0 * qty * default_metadata().maker_fee;
        let expected_cash = 10000.0 - expected_margin - expected_fee;

        assert_snap_eq(cash_after, expected_cash);
    }

    // 验证分数数量完整开平一轮后，现金严格符合“初始资金+利润-开平手续费”公式。
    #[tokio::test]
    async fn open_close_round_trip_fractional_cash_matches_formula() {
        let qty = 0.333;
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();

        let open_fee = 105.0 * qty * default_metadata().taker_fee;
        let close_fee = 110.0 * qty * default_metadata().taker_fee;
        let profit = (110.0 - 105.0) * qty;
        let expected_cash = 10000.0 + profit - open_fee - close_fee;

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_snap_eq(cash_after, expected_cash);
    }

    // 验证逐根 next 的状态严格一致：现金、权益、仓位、挂单和历史记录均符合精确公式。
    #[tokio::test]
    async fn strict_step_by_step_state_assertions_on_each_next() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.1, 101.2, 99.4, 100.5),
                gen_kline(2, 105.3, 106.4, 104.2, 105.9),
                gen_kline(3, 110.2, 111.4, 109.3, 110.8),
                gen_kline(4, 120.1, 121.5, 119.6, 120.4),
            ],
        );

        let md = default_metadata();
        let qty = 1.0;
        let leverage = 10.0;
        let open_fill_price = 105.3;
        let bar2_close = 105.9;
        let bar3_close = 110.8;
        let close_fill_price = 120.1;

        // Step 1: 第一根 K 线，仅推进时间，不应有资金与仓位变化。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), 10000.0);
        assert_snap_eq(exchange.get_equity().await.unwrap(), 10000.0);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            exchange
                .get_history_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );

        // 下市价开多单：撮合前资金不变，仅产生 pending。
        let open_id = exchange.buy(SYMBOL, 1.0).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), 10000.0);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );

        // Step 2: 第二根 K 线撮合开仓。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&open_id).await.unwrap().unwrap();
        let position_after_open = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let pending_after_open = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        let open_margin = open_fill_price * qty / leverage;
        let open_fee = open_fill_price * qty * md.taker_fee;
        let expected_cash_after_open = 10000.0 - open_margin - open_fee;
        let expected_equity_after_open =
            expected_cash_after_open + open_margin + (bar2_close - open_fill_price) * qty;
        let expected_liquidation_price = open_fill_price * (1.0 - 1.0 / leverage + md.maintenance);

        assert_eq!(open_order.status, Status::Filled);
        assert_snap_eq(open_order.avg_price, open_fill_price);
        assert_snap_eq(open_order.cumulative_quantity, qty);

        assert_eq!(position_after_open.side, Side::Buy);
        assert_eq!(position_after_open.leverage, 10);
        assert_snap_eq(position_after_open.open_avg_price, open_fill_price);
        assert_snap_eq(position_after_open.quantity, qty);
        assert_snap_eq(position_after_open.margin, open_margin);
        assert_snap_eq(
            position_after_open.profit,
            (bar2_close - open_fill_price) * qty,
        );
        assert_snap_eq(
            position_after_open.liquidation_price,
            expected_liquidation_price,
        );

        assert_snap_eq(exchange.get_cash().await.unwrap(), expected_cash_after_open);
        assert_snap_eq(
            exchange.get_equity().await.unwrap(),
            expected_equity_after_open,
        );

        assert_eq!(pending_after_open.len(), 1);
        assert_eq!(pending_after_open[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_open[0].side, Side::Sell);
        assert_snap_eq(pending_after_open[0].price, expected_liquidation_price);

        // Step 3: 第三根 K 线无成交，仅更新浮盈。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_after_bar3 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let expected_cash_after_bar3 = expected_cash_after_open;
        let expected_equity_after_bar3 =
            expected_cash_after_bar3 + open_margin + (bar3_close - open_fill_price) * qty;

        assert_snap_eq(exchange.get_cash().await.unwrap(), expected_cash_after_bar3);
        assert_snap_eq(
            exchange.get_equity().await.unwrap(),
            expected_equity_after_bar3,
        );
        assert_snap_eq(
            position_after_bar3.profit,
            (bar3_close - open_fill_price) * qty,
        );
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );

        // 提交 reduce-only 平仓单：撮合前资金不变，挂单增加。
        let close_id = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), expected_cash_after_bar3);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // Step 4: 第四根 K 线撮合平仓。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash_after_close = exchange.get_cash().await.unwrap();
        let equity_after_close = exchange.get_equity().await.unwrap();
        let pending_after_close = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        let close_fee = close_fill_price * qty * md.taker_fee;
        let close_profit = (close_fill_price - open_fill_price) * qty;
        let expected_cash_after_close =
            expected_cash_after_bar3 - close_fee + open_margin + close_profit;

        assert_eq!(close_order.status, Status::Filled);
        assert_snap_eq(close_order.avg_price, close_fill_price);
        assert_snap_eq(close_order.cumulative_quantity, qty);

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(pending_after_close.is_empty());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_snap_eq(history[0].open_avg_price, open_fill_price);
        assert_snap_eq(history[0].close_avg_price, close_fill_price);
        assert_snap_eq(history[0].profit, close_profit);
        assert_snap_eq(history[0].fee, open_fee + close_fee);
        assert_snap_eq(history[0].total_profit, close_profit - open_fee - close_fee);

        assert_snap_eq(cash_after_close, expected_cash_after_close);
        assert_snap_eq(equity_after_close, expected_cash_after_close);
    }

    // 验证复杂路径：市价开仓 -> 调杠杆 -> 加减保证金 -> 条件单触发 -> 反向开仓 -> 限价减仓，
    // 且每次 next 后均对 cash/equity/position/pending/history 做精确小数断言。
    #[tokio::test]
    async fn strict_complex_flow_asserts_all_states_on_every_next() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.1, 101.2, 99.4, 100.5),
                gen_kline(2, 105.3, 106.4, 104.2, 105.9),
                gen_kline(3, 110.2, 111.4, 109.3, 110.8),
                gen_kline(4, 108.4, 109.1, 107.8, 108.0),
                gen_kline(5, 103.7, 104.2, 102.9, 103.1),
            ],
        )
        .cash(20000.0);

        let md = default_metadata();

        let qty_long = 1.234;
        let qty_reverse_order = 2.0;
        let qty_reverse_short = qty_reverse_order - qty_long;
        let qty_limit_reduce = 0.3;

        let p_open_long = 105.3;
        let p_close_bar2 = 105.9;
        let p_open_reverse = 110.5;
        let p_close_bar4 = 108.0;
        let p_limit_close = 104.2;
        let p_close_bar5 = 103.1;

        // Step 1: 仅推进时间，账户初始状态不变。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), 20000.0);
        assert_snap_eq(exchange.get_equity().await.unwrap(), 20000.0);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            exchange
                .get_history_position_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );

        // 下市价开多：撮合前仅新增挂单。
        let market_open_id = exchange.buy(SYMBOL, qty_long).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), 20000.0);
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            1
        );

        // Step 2: 市价开多在 bar2 成交。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let open_order = exchange.get_order(&market_open_id).await.unwrap().unwrap();
        let mut position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let pending_after_open = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        let open_margin_10 = p_open_long * qty_long / 10.0;
        let open_fee = p_open_long * qty_long * md.taker_fee;
        let cash_after_open = 20000.0 - open_margin_10 - open_fee;
        let upnl_bar2 = (p_close_bar2 - p_open_long) * qty_long;
        let equity_after_open = cash_after_open + open_margin_10 + upnl_bar2;
        let liq_after_open = p_open_long * (1.0 - 1.0 / 10.0 + md.maintenance);

        assert_eq!(open_order.status, Status::Filled);
        assert_snap_eq(open_order.avg_price, p_open_long);
        assert_snap_eq(open_order.cumulative_quantity, qty_long);

        assert_eq!(position.side, Side::Buy);
        assert_eq!(position.leverage, 10);
        assert_snap_eq(position.open_avg_price, p_open_long);
        assert_snap_eq(position.quantity, qty_long);
        assert_snap_eq(position.margin, open_margin_10);
        assert_snap_eq(position.profit, upnl_bar2);
        assert_snap_eq(position.liquidation_price, liq_after_open);

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_open);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_after_open);
        assert_eq!(pending_after_open.len(), 1);
        assert_eq!(pending_after_open[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_open[0].side, Side::Sell);
        assert_snap_eq(pending_after_open[0].price, liq_after_open);

        // 调杠杆 + 追加/减少保证金（不经过 next，但会影响后续 next 断言基线）。
        exchange.set_leverage(SYMBOL, 8).await.unwrap();
        exchange
            .append_position_margin(SYMBOL, 1.111)
            .await
            .unwrap();
        exchange
            .append_position_margin(SYMBOL, -0.321)
            .await
            .unwrap();

        let margin_after_leverage = p_open_long * qty_long / 8.0;
        let cash_after_leverage = cash_after_open - (margin_after_leverage - open_margin_10);
        let margin_after_adjust = margin_after_leverage + 1.111 - 0.321;
        let cash_after_adjust = cash_after_leverage - 1.111 + 0.321;
        let liq_after_adjust = p_open_long * (1.0 - 1.0 / 8.0 + md.maintenance)
            - (margin_after_adjust - margin_after_leverage) / qty_long;

        let position_after_adjust = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position_after_adjust.leverage, 8);
        assert_snap_eq(position_after_adjust.margin, margin_after_adjust);
        assert_snap_eq(position_after_adjust.liquidation_price, liq_after_adjust);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_adjust);

        // 下触发市价卖单（条件单），将来触发后反向开仓。
        let trigger_id = exchange
            .sell_trigger_market(SYMBOL, 110.5, qty_reverse_order)
            .await
            .unwrap();
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );

        // Step 3: 条件单触发并撮合成交
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let fee_full_reverse_order = p_open_reverse * qty_reverse_order * md.taker_fee;
        let close_profit_long = (p_open_reverse - p_open_long) * qty_long;
        let reverse_margin_8 = p_open_reverse * qty_reverse_short / 8.0;

        let cash_after_reverse =
            cash_after_adjust - fee_full_reverse_order + margin_after_adjust + close_profit_long
                - reverse_margin_8;
        let upnl_bar4_short = (p_open_reverse - p_close_bar4) * qty_reverse_short;
        let equity_bar4 = cash_after_reverse + reverse_margin_8 + upnl_bar4_short;
        let liq_short = p_open_reverse * (1.0 + 1.0 / 8.0 - md.maintenance);

        let history_after_reverse = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let pending_after_reverse = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 8);
        assert_snap_eq(position.open_avg_price, p_open_reverse);
        assert_snap_eq(position.quantity, qty_reverse_short);
        assert_snap_eq(position.margin, reverse_margin_8);
        assert_snap_eq(position.profit, upnl_bar4_short);
        assert_snap_eq(position.liquidation_price, liq_short);

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_reverse);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_bar4);

        assert_eq!(history_after_reverse.len(), 1);
        assert_eq!(history_after_reverse[0].side, Side::Buy);
        assert_snap_eq(history_after_reverse[0].open_avg_price, p_open_long);
        assert_snap_eq(history_after_reverse[0].close_avg_price, p_open_reverse);
        assert_snap_eq(history_after_reverse[0].close_quantity, qty_long);
        assert_snap_eq(history_after_reverse[0].profit, close_profit_long);

        assert_eq!(pending_after_reverse.len(), 1);
        assert_eq!(pending_after_reverse[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_reverse[0].side, Side::Buy);
        assert_snap_eq(pending_after_reverse[0].price, liq_short);

        // 下 reduce-only 限价买单，对空仓做部分平仓。
        let limit_reduce_id = exchange
            .buy_limit_reduce_only(SYMBOL, 104.0, qty_limit_reduce)
            .await
            .unwrap();
        assert_eq!(
            exchange.get_pending_order_list(SYMBOL).await.unwrap().len(),
            2
        );
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_reverse);

        // Step 5: 限价减仓成交（买价>=open，按 high 成交），并更新历史与仓位。
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_limit_order = exchange.get_order(&limit_reduce_id).await.unwrap().unwrap();
        let history_after_limit = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let pending_after_limit = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

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
        assert_snap_eq(close_limit_order.avg_price, p_limit_close);
        assert_snap_eq(close_limit_order.cumulative_quantity, qty_limit_reduce);

        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 8);
        assert_snap_eq(position.open_avg_price, p_open_reverse);
        assert_snap_eq(position.quantity, qty_short_left);
        assert_snap_eq(position.margin, margin_short_left);
        assert_snap_eq(position.profit, upnl_bar5_short);
        assert_snap_eq(position.liquidation_price, liq_short);

        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_after_limit);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_bar5);

        assert_eq!(pending_after_limit.len(), 1);
        assert_eq!(pending_after_limit[0].kind, Kind::Liquidation);
        assert_eq!(pending_after_limit[0].side, Side::Buy);
        assert_snap_eq(pending_after_limit[0].price, liq_short);

        assert_eq!(history_after_limit.len(), 2);
        assert_eq!(history_after_limit[1].side, Side::Sell);
        assert_snap_eq(history_after_limit[1].open_avg_price, p_open_reverse);
        assert_snap_eq(history_after_limit[1].close_avg_price, p_limit_close);
        assert_snap_eq(history_after_limit[1].close_quantity, qty_limit_reduce);
        assert_snap_eq(history_after_limit[1].profit, close_profit_short);
    }

    // // 验证“复杂到令人发指”的状态机：
    // // 市价+限价混合开仓 -> 调杠杆/加减保证金 -> 条件市价触发并反向 -> 条件限价触发并成交 ->
    // // 调杠杆失败与恢复 -> 再次调杠杆与保证金调整 -> 市价全平，且每个 next 后精确断言全状态。
    // #[tokio::test]
    // async fn insane_state_machine_asserts_everything_each_next() {
    //     let exchange = test_exchange_with(
    //         default_metadata(),
    //         vec![
    //             gen_kline(1, 100.17, 101.23, 99.41, 100.58),
    //             gen_kline(2, 105.37, 106.48, 104.26, 105.91),
    //             gen_kline(3, 110.29, 111.44, 109.35, 110.83),
    //             gen_kline(4, 108.42, 109.10, 107.80, 108.00),
    //             gen_kline(5, 103.77, 104.20, 102.90, 103.10),
    //             gen_kline(6, 99.33, 100.50, 98.60, 99.80),
    //             gen_kline(7, 112.66, 113.20, 111.90, 112.20),
    //         ],
    //     )
    //     .cash(30000.0);

    //     let md = default_metadata();

    //     let q_market_open = 1.2345;
    //     let q_limit_open = 0.5;
    //     let q_long = q_market_open + q_limit_open;
    //     let q_reverse_order = 3.0;
    //     let q_short = q_reverse_order - q_long;
    //     let q_trigger_limit_close = 0.6;
    //     let q_short_left = q_short - q_trigger_limit_close;

    //     let p_market_open = 105.37;
    //     let p_limit_open = 104.70;
    //     let p_reverse_open = 108.42;
    //     let p_trigger_limit_close = 100.50;
    //     let p_final_close = 112.66;

    //     // Step 1: 初始化 next。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), 30000.0);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), 30000.0);
    //     assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    //     assert!(
    //         exchange
    //             .get_pending_order_list(SYMBOL)
    //             .await
    //             .unwrap()
    //             .is_empty()
    //     );

    //     // 提交市价开多 + 限价开多 + 条件市价反向单（trigger=110.5）。
    //     let id_market_open = exchange.buy(SYMBOL, q_market_open).await.unwrap();
    //     let id_limit_open = exchange
    //         .buy_limit(SYMBOL, p_limit_open, q_limit_open)
    //         .await
    //         .unwrap();
    //     let id_trigger_reverse = exchange
    //         .sell_trigger_market(SYMBOL, 110.5, q_reverse_order)
    //         .await
    //         .unwrap();

    //     // Step 2: 市价与限价开仓成交，trigger 仍未触发。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let order_market_open = exchange.get_order(&id_market_open).await.unwrap().unwrap();
    //     let order_limit_open = exchange.get_order(&id_limit_open).await.unwrap().unwrap();
    //     let trigger_reverse = exchange
    //         .get_order(&id_trigger_reverse)
    //         .await
    //         .unwrap()
    //         .unwrap();
    //     let mut position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
    //     let pending_s2 = exchange.get_pending_order_list(SYMBOL).await.unwrap();

    //     let avg_long = (q_market_open * p_market_open + q_limit_open * p_limit_open) / q_long;
    //     let margin_s2 = q_market_open * p_market_open / 10.0 + q_limit_open * p_limit_open / 10.0;
    //     let fee_open_market = q_market_open * p_market_open * md.taker_fee;
    //     let fee_open_limit = q_limit_open * p_limit_open * md.maker_fee;
    //     let cash_s2 = 30000.0 - margin_s2 - fee_open_market - fee_open_limit;
    //     let upnl_s2 = (105.91 - avg_long) * q_long;
    //     let equity_s2 = cash_s2 + margin_s2 + upnl_s2;

    //     assert_eq!(order_market_open.status, Status::Filled);
    //     assert_eq!(order_limit_open.status, Status::Filled);
    //     assert_eq!(trigger_reverse.status, Status::Submitted);
    //     assert_eq!(position.side, Side::Buy);
    //     assert_eq!(position.leverage, 10);
    //     assert_snap_eq(position.open_avg_price, avg_long);
    //     assert_snap_eq(position.quantity, q_long);
    //     assert_snap_eq(position.margin, margin_s2);
    //     assert_snap_eq(position.profit, upnl_s2);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s2);
    //     assert_eq!(pending_s2.len(), 2);
    //     assert!(pending_s2.iter().any(|o| o.kind == Kind::Liquidation));
    //     assert!(pending_s2.iter().any(|o| o.kind == Kind::Trigger));

    //     // 有普通挂单（Trigger）时调杠杆应失败。
    //     let lev_err_with_trigger = exchange.set_leverage(SYMBOL, 8).await.unwrap_err();
    //     assert!(lev_err_with_trigger.to_string().contains("pending orders"));

    //     // 新增条件限价平空单（将在后面触发并成交）。
    //     let id_trigger_limit = exchange
    //         .buy_trigger_limit(SYMBOL, 103.0, 103.2, q_trigger_limit_close)
    //         .await
    //         .unwrap();

    //     // Step 3: 条件市价单触发（转 Market），仓位仍是多。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let trigger_reverse_s3 = exchange
    //         .get_order(&id_trigger_reverse)
    //         .await
    //         .unwrap()
    //         .unwrap();
    //     let trigger_limit_s3 = exchange
    //         .get_order(&id_trigger_limit)
    //         .await
    //         .unwrap()
    //         .unwrap();
    //     let pending_s3 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
    //     position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

    //     let upnl_s3 = (110.83 - avg_long) * q_long;
    //     let equity_s3 = cash_s2 + margin_s2 + upnl_s3;

    //     // 条件单触发后立即撮合，平多开空（现在在 Step 3 完成）
    //     assert_eq!(trigger_reverse_s3.status, Status::Filled);
    //     assert_eq!(trigger_limit_s3.status, Status::Submitted);
    //     assert_eq!(position.side, Side::Sell);
    //     assert_eq!(position.leverage, 10);
    //     assert_snap_eq(position.quantity, q_short);
    //     assert_snap_eq(position.margin, margin_s2);
    //     assert_snap_eq(position.profit, upnl_s3);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s3);
    //     assert_eq!(pending_s3.len(), 3);
    //     assert!(pending_s3.iter().any(|o| o.kind == Kind::Liquidation));
    //     assert!(pending_s3.iter().any(|o| o.kind == Kind::Market));
    //     assert!(pending_s3.iter().any(|o| o.kind == Kind::Trigger));

    //     // Step 4: Market 成交，先平多再反向开空。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let fee_reverse_full = p_reverse_open * q_reverse_order * md.taker_fee;
    //     let close_profit_long = (p_reverse_open - avg_long) * q_long;
    //     let margin_short_s4 = p_reverse_open * q_short / 10.0;
    //     let cash_s4 = cash_s2 - fee_reverse_full + margin_s2 + close_profit_long - margin_short_s4;
    //     let upnl_s4 = (p_reverse_open - 108.0) * q_short;
    //     let equity_s4 = cash_s4 + margin_short_s4 + upnl_s4;

    //     let pending_s4 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
    //     let history_s4 = exchange.get_history_position_list(SYMBOL).await.unwrap();
    //     position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

    //     assert_eq!(position.side, Side::Sell);
    //     assert_eq!(position.leverage, 10);
    //     assert_snap_eq(position.open_avg_price, p_reverse_open);
    //     assert_snap_eq(position.quantity, q_short);
    //     assert_snap_eq(position.margin, margin_short_s4);
    //     assert_snap_eq(position.profit, upnl_s4);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s4);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s4);
    //     assert_eq!(history_s4.len(), 1);
    //     assert_eq!(history_s4[0].side, Side::Buy);
    //     assert_eq!(pending_s4.len(), 2);
    //     assert!(pending_s4.iter().any(|o| o.kind == Kind::Liquidation));
    //     assert!(pending_s4.iter().any(|o| o.kind == Kind::Trigger));

    //     // Step 5: 条件限价触发（转 Limit），空仓仍未变化。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let trigger_limit_s5 = exchange
    //         .get_order(&id_trigger_limit)
    //         .await
    //         .unwrap()
    //         .unwrap();
    //     let pending_s5 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
    //     position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

    //     let upnl_s5 = (p_reverse_open - 103.10) * q_short;
    //     let freeze_trigger_limit = 103.2 * q_trigger_limit_close / 10.0;
    //     let cash_s5 = cash_s4 - freeze_trigger_limit;
    //     let equity_s5 = cash_s5 + margin_short_s4 + upnl_s5 + freeze_trigger_limit;

    //     assert_eq!(trigger_limit_s5.status, Status::Filled);
    //     assert_eq!(position.side, Side::Sell);
    //     assert_snap_eq(position.quantity, q_short);
    //     assert_snap_eq(position.margin, margin_short_s4);
    //     assert_snap_eq(position.profit, upnl_s5);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s5);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s5);
    //     assert_eq!(pending_s5.len(), 2);
    //     assert!(pending_s5.iter().any(|o| o.kind == Kind::Liquidation));
    //     assert!(pending_s5.iter().any(|o| o.kind == Kind::Limit));

    //     // Step 6: 触发后的限价单成交，部分平空。
    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let converted_limit = exchange
    //         .get_history_order_list(SYMBOL)
    //         .await
    //         .unwrap()
    //         .into_iter()
    //         .find(|o| {
    //             o.kind == Kind::Limit
    //                 && o.status == Status::Filled
    //                 && o.side == Side::Buy
    //                 && o.quantity.snap_eq(q_trigger_limit_close, SNAP_TICK)
    //         })
    //         .unwrap();

    //     let close_margin_s6 = margin_short_s4 * (q_trigger_limit_close / q_short);
    //     let close_profit_s6 = (p_reverse_open - p_trigger_limit_close) * q_trigger_limit_close;
    //     let close_fee_s6 = p_trigger_limit_close * q_trigger_limit_close * md.maker_fee;
    //     let cash_s6 = cash_s4 - close_fee_s6 + close_margin_s6 + close_profit_s6;
    //     let margin_s6 = margin_short_s4 - close_margin_s6;
    //     let upnl_s6 = (p_reverse_open - 99.80) * q_short_left;
    //     let equity_s6 = cash_s6 + margin_s6 + upnl_s6;

    //     let pending_s6 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
    //     position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

    //     assert_eq!(converted_limit.side, Side::Buy);
    //     assert_snap_eq(converted_limit.avg_price, p_trigger_limit_close);
    //     assert_eq!(position.side, Side::Sell);
    //     assert_eq!(position.leverage, 10);
    //     assert_snap_eq(position.quantity, q_short_left);
    //     assert_snap_eq(position.margin, margin_s6);
    //     assert_snap_eq(position.profit, upnl_s6);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s6);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s6);
    //     assert_eq!(pending_s6.len(), 1);
    //     assert_eq!(pending_s6[0].kind, Kind::Liquidation);

    //     // 带普通挂单时调杠杆应失败。
    //     let id_block_lev = exchange
    //         .sell_limit_reduce_only(SYMBOL, 130.0, 0.1)
    //         .await
    //         .unwrap();
    //     let lev_err = exchange.set_leverage(SYMBOL, 5).await.unwrap_err();
    //     assert!(lev_err.to_string().contains("pending orders"));

    //     // 撤掉普通挂单后允许调杠杆，并再做保证金调整。
    //     exchange.cancel_order(SYMBOL, &id_block_lev).await.unwrap();
    //     exchange.set_leverage(SYMBOL, 5).await.unwrap();
    //     exchange
    //         .append_position_margin(SYMBOL, 0.333)
    //         .await
    //         .unwrap();
    //     exchange.append_position_margin(SYMBOL, -0.2).await.unwrap();

    //     let margin_lev5 = p_reverse_open * q_short_left / 5.0;
    //     let cash_lev5 = cash_s6 - (margin_lev5 - margin_s6);
    //     let margin_lev5_adj = margin_lev5 + 0.333 - 0.2;
    //     let cash_lev5_adj = cash_lev5 - 0.333 + 0.2;

    //     let position_after_adj = exchange.get_position(SYMBOL).await.unwrap().unwrap();
    //     assert_eq!(position_after_adj.leverage, 5);
    //     assert_snap_eq(position_after_adj.margin, margin_lev5_adj);
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_lev5_adj);

    //     // close_all 下市价平仓单，Step7 全平并校验最终资金与历史。
    //     exchange.close_all_position(SYMBOL).await.unwrap();

    //     exchange.next(SYMBOL, Level::Minute1).await.unwrap();

    //     let fee_final = p_final_close * q_short_left * md.taker_fee;
    //     let profit_final = (p_reverse_open - p_final_close) * q_short_left;
    //     let cash_final = cash_lev5_adj - fee_final + margin_lev5_adj + profit_final;

    //     let history_final = exchange.get_history_position_list(SYMBOL).await.unwrap();
    //     let pending_final = exchange.get_pending_order_list(SYMBOL).await.unwrap();

    //     assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    //     assert!(pending_final.is_empty());
    //     assert_snap_eq(exchange.get_cash().await.unwrap(), cash_final);
    //     assert_snap_eq(exchange.get_equity().await.unwrap(), cash_final);

    //     assert_eq!(history_final.len(), 2);
    //     assert_eq!(history_final[0].side, Side::Buy);
    //     assert_eq!(history_final[1].side, Side::Sell);
    //     assert_snap_eq(history_final[1].open_avg_price, p_reverse_open);
    //     assert_snap_eq(history_final[1].close_avg_price, p_final_close);
    //     assert_snap_eq(history_final[1].close_quantity, q_short);
    //     assert_snap_eq(history_final[1].max_quantity, q_short);
    // }

    #[tokio::test]
    async fn insane_state_machine_asserts_everything_each_next() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.1, 101.2, 99.4, 100.5),
                gen_kline(2, 105.3, 106.4, 104.2, 105.9),
                gen_kline(3, 110.2, 111.4, 109.3, 110.8),
                gen_kline(4, 108.4, 109.1, 107.8, 108.0),
                gen_kline(5, 103.7, 104.2, 102.9, 103.1),
                gen_kline(6, 99.3, 100.5, 98.6, 99.8),
                gen_kline(7, 112.6, 113.2, 111.9, 112.2),
            ],
        )
        .cash(30000.0);

        let md = default_metadata();

        let q_market_open = 1.23;
        let q_limit_open = 0.5;
        let q_long = q_market_open + q_limit_open; // 1.7345
        let q_reverse_order = 3.0;
        let q_short = q_reverse_order - q_long; // 1.2655
        let q_trigger_limit_close = 0.6;
        let q_short_left = q_short - q_trigger_limit_close; // 0.6655

        let p_market_open = 105.3;
        let p_limit_open = 104.7;
        let p_trigger_reverse = 110.5; // 条件市价单触发价
        let p_trigger_limit = 103.2; // 条件限价单成交价
        let p_final_close = 112.6;

        // ---------- Step 1: 初始状态 ----------
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        assert_snap_eq(exchange.get_cash().await.unwrap(), 30000.0);
        assert_snap_eq(exchange.get_equity().await.unwrap(), 30000.0);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );

        // 提交市价开多 + 限价开多 + 条件市价反向单（trigger=110.5）
        let id_market_open = exchange.buy(SYMBOL, q_market_open).await.unwrap();
        let id_limit_open = exchange
            .buy_limit(SYMBOL, p_limit_open, q_limit_open)
            .await
            .unwrap();
        let id_trigger_reverse = exchange
            .sell_trigger_market(SYMBOL, p_trigger_reverse, q_reverse_order)
            .await
            .unwrap();

        // ---------- Step 2: 市价与限价开仓成交，trigger 未触发 ----------
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order_market_open = exchange.get_order(&id_market_open).await.unwrap().unwrap();
        let order_limit_open = exchange.get_order(&id_limit_open).await.unwrap().unwrap();
        let trigger_reverse = exchange
            .get_order(&id_trigger_reverse)
            .await
            .unwrap()
            .unwrap();
        let mut position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let pending_s2 = exchange.get_pending_order_list(SYMBOL).await.unwrap();

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
        assert_snap_eq(position.open_avg_price, avg_long);
        assert_snap_eq(position.quantity, q_long);
        assert_snap_eq(position.margin, margin_long);
        assert_snap_eq(position.profit, upnl_s2);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s2);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s2);
        assert_eq!(pending_s2.len(), 2);
        assert!(pending_s2.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s2.iter().any(|o| o.kind == Kind::Trigger));

        // 有普通挂单（Trigger）时调杠杆应失败
        let lev_err_with_trigger = exchange.set_leverage(SYMBOL, 8).await.unwrap_err();
        assert!(lev_err_with_trigger.to_string().contains("pending orders"));

        // 新增条件限价平空单（将在后面触发并成交）
        let id_trigger_limit = exchange
            .buy_trigger_limit(SYMBOL, 103.0, p_trigger_limit, q_trigger_limit_close)
            .await
            .unwrap();

        // ---------- Step 3: 条件市价单触发并立即成交（平多开空） ----------
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

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
        let pending_s3 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

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
        assert_snap_eq(position.open_avg_price, p_trigger_reverse);
        assert_snap_eq(position.quantity, q_short);
        assert_snap_eq(position.margin, margin_short);
        assert_snap_eq(position.profit, upnl_s3);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s3);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s3);
        assert_eq!(pending_s3.len(), 2);
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s3.iter().any(|o| o.kind != Kind::Market));
        assert!(pending_s3.iter().any(|o| o.kind == Kind::Trigger));

        // ---------- Step 4: 下一根K线，更新未实现盈亏，条件限价单尚未触发 ----------
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_limit_s4 = exchange
            .get_order(&id_trigger_limit)
            .await
            .unwrap()
            .unwrap();
        let pending_s4 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        let history_s4 = exchange.get_history_position_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let current_price_s4 = 108.0; // K线4收盘价
        let upnl_s4 = (p_trigger_reverse - current_price_s4) * q_short;
        let equity_s4 = cash_s3 + margin_short + upnl_s4;

        assert_eq!(trigger_limit_s4.status, Status::Submitted); // 尚未触发
        assert_eq!(position.side, Side::Sell);
        assert_eq!(position.leverage, 10);
        assert_snap_eq(position.open_avg_price, p_trigger_reverse);
        assert_snap_eq(position.quantity, q_short);
        assert_snap_eq(position.margin, margin_short);
        assert_snap_eq(position.profit, upnl_s4);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s3);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s4);
        assert_eq!(history_s4.len(), 1);
        assert_eq!(history_s4[0].side, Side::Buy);
        assert_eq!(pending_s4.len(), 2);
        assert!(pending_s4.iter().any(|o| o.kind == Kind::Liquidation));
        assert!(pending_s4.iter().any(|o| o.kind == Kind::Trigger));

        // ---------- Step 5: 条件限价单触发，转为限价单并冻结保证金 ----------
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_limit_s5 = exchange
            .get_order(&id_trigger_limit)
            .await
            .unwrap()
            .unwrap();
        let pending_s5 = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let freeze_margin = p_trigger_limit * q_trigger_limit_close / 10.0;
        let upnl_s5 = (p_trigger_reverse - 103.1) * q_short_left;
        let close_margin = margin_short * (q_trigger_limit_close / q_short);
        let margin_short = margin_short - close_margin;
        let close_profit = (p_trigger_reverse - p_trigger_limit) * q_trigger_limit_close;
        let close_fee = p_trigger_limit * q_trigger_limit_close * md.maker_fee;
        let cash_s5 = cash_s3 + close_margin + close_profit - close_fee;
        let equity_s5 = cash_s5 + margin_short + upnl_s5;

        assert_eq!(trigger_limit_s5.status, Status::Filled); // 触发单转为限价单
        assert_eq!(position.side, Side::Sell);
        assert_snap_eq(position.open_avg_price, p_trigger_reverse);
        assert_snap_eq(position.quantity, q_short_left);
        assert_snap_eq(position.margin, margin_short);
        assert_snap_eq(position.profit, upnl_s5);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_s5);
        assert_snap_eq(exchange.get_equity().await.unwrap(), equity_s5);
        assert_eq!(pending_s5.len(), 1);
        assert!(pending_s5.iter().any(|o| o.kind == Kind::Liquidation));

        // ---------- Step 6 ----------
        println!("margin_short: {}", margin_short);
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // ---------- 调整杠杆和保证金 ----------
        // 带普通挂单时调杠杆应失败
        let id_block_lev = exchange
            .sell_limit_reduce_only(SYMBOL, 130.0, 0.1)
            .await
            .unwrap();
        let lev_err = exchange.set_leverage(SYMBOL, 5).await.unwrap_err();
        assert!(lev_err.to_string().contains("pending orders"));

        // 撤掉普通挂单后允许调杠杆，并做保证金调整
        exchange.cancel_order(SYMBOL, &id_block_lev).await.unwrap();
        exchange.set_leverage(SYMBOL, 5).await.unwrap();
        exchange
            .append_position_margin(SYMBOL, 0.333)
            .await
            .unwrap();
        exchange.append_position_margin(SYMBOL, -0.2).await.unwrap();

        let append_margin = p_trigger_reverse * q_short_left / 5.0 - margin_short;
        let cash_lev5_adj = cash_s5 - append_margin - 0.333 + 0.2;
        let margin_lev5_adj = margin_short + append_margin + 0.333 - 0.2;

        let position_after_adj = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        println!("position_after_adj.margin: {}", position_after_adj.margin);
        println!("append_margin: {}", append_margin);

        assert_eq!(position_after_adj.leverage, 5);
        assert_snap_eq(position_after_adj.margin, margin_lev5_adj);
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_lev5_adj);

        // ---------- 市价全平 ----------
        exchange.close_all_position(SYMBOL).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let fee_final = p_final_close * q_short_left * md.taker_fee;
        let profit_final = (p_trigger_reverse - p_final_close) * q_short_left;
        let cash_final = cash_lev5_adj - fee_final + margin_lev5_adj + profit_final;

        let history_final = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let pending_final = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(pending_final.is_empty());
        assert_snap_eq(exchange.get_cash().await.unwrap(), cash_final);
        assert_snap_eq(exchange.get_equity().await.unwrap(), cash_final);

        assert_eq!(history_final.len(), 2);
        assert_eq!(history_final[0].side, Side::Buy);
        assert_eq!(history_final[1].side, Side::Sell);
        assert_snap_eq(history_final[1].open_avg_price, p_trigger_reverse);
        assert_snap_eq(history_final[1].close_avg_price, p_final_close);
        assert_snap_eq(history_final[1].close_quantity, q_short);
        assert_snap_eq(history_final[1].max_quantity, q_short);
    }

    // 验证限价单成交时若手续费不足被拒绝，会返还此前冻结的保证金。
    #[tokio::test]
    async fn limit_order_rejected_on_fee_shortage_refunds_frozen_margin() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(10.51)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 105.0, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_snap_eq(cash_after, cash_before);
    }

    // 验证按 ID 撤单不会允许撤掉强平保护单。
    #[tokio::test]
    async fn cancel_order_rejects_liquidation_order() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_id = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .map(|v| v.id)
            .unwrap();

        let result = exchange.cancel_order(SYMBOL, &liq_id).await.unwrap_err();

        assert!(result.to_string().contains("non-normal order"));

        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        assert!(pending.iter().any(|v| v.kind == Kind::Liquidation));
    }

    // 验证 cancel_order 会把订单以 Canceled 状态写入历史，便于后续审计。
    #[tokio::test]
    async fn cancel_order_writes_canceled_status_to_history() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let id = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        exchange.cancel_order(SYMBOL, &id).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Canceled);
    }

    // 验证最小名义价值对市价单在成交时校验：满足阈值则可成交。
    #[tokio::test]
    async fn market_order_min_notional_is_checked_at_fill_time_and_can_pass() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
    }

    // 验证最小名义价值对市价单在成交时校验：不满足阈值则拒绝。
    #[tokio::test]
    async fn market_order_min_notional_is_checked_at_fill_time_and_can_reject() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 0.5).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Rejected);
    }

    // 验证市价单在 min_notional 成交拒绝时会返还冻结保证金，现金保持不变。
    #[tokio::test]
    async fn market_order_min_notional_reject_refunds_frozen_margin() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy(SYMBOL, 0.5).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_snap_eq(cash_after, cash_before);
    }

    // 验证触发限价单在触发后会按限价成交前校验最小名义价值，不满足时拒绝。
    #[tokio::test]
    async fn trigger_limit_order_min_notional_is_checked_at_fill_time_and_can_reject() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;
        metadata.min_size = 0.00000001;

        let exchange = test_exchange_with(
            metadata,
            vec![
                gen_kline(1, 10.0, 10.5, 9.5, 10.0),
                gen_kline(2, 20.0, 21.0, 19.0, 20.0),
                gen_kline(3, 20.0, 21.0, 19.0, 20.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(SYMBOL, 20.0, 20.0, 4.0)
            .await
            .unwrap();

        // 第 2 根触发并立即执行，名义价值不足应被拒绝
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        // 触发单状态应为 Filled（已触发）
        assert_eq!(order.status, Status::Filled);

        // 检查所有历史订单
        let history = exchange.get_history_order_list(SYMBOL).await.unwrap();
        println!("History orders count: {}", history.len());
        for (i, o) in history.iter().enumerate() {
            println!(
                "Order {}: id={}, kind={:?}, status={:?}, price={}, quantity={}, avg_price={}, notional={}",
                i,
                o.id,
                o.kind,
                o.status,
                o.price,
                o.quantity,
                o.avg_price,
                o.avg_price * o.quantity
            );
        }

        // 检查是否有仓位
        let position = exchange.get_position(SYMBOL).await.unwrap();
        println!("Position: {:?}", position);

        // 转换的限价单因名义价值不足被拒绝
        let converted_limit = history
            .iter()
            .find(|v| v.kind == Kind::Limit && v.status == Status::Rejected)
            .cloned();
        assert!(
            converted_limit.is_some(),
            "Should have a rejected limit order"
        );
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证触发限价单立即执行，在触发后满足最小名义价值时可以正常成交。
    #[tokio::test]
    async fn trigger_limit_order_min_notional_is_checked_at_fill_time_and_can_pass() {
        let mut metadata = default_metadata();
        metadata.min_notional = 100.0;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange
            .buy_trigger_limit(SYMBOL, 105.0, 110.0, 1.0)
            .await
            .unwrap();

        // 第 2 根触发并立即执行（限价110≥开盘价105，以最高价106成交）
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let trigger_order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.open_avg_price, 106.0); // 立即执行，以当前K线最高价成交
        assert_snap_eq(position.quantity, 1.0);
    }

    // 验证权益口径为 cash + margin + upnl。
    #[tokio::test]
    async fn equity_includes_cash_margin_and_upnl() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash = exchange.get_cash().await.unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let equity = exchange.get_equity().await.unwrap();

        assert_snap_eq(equity, cash + position.margin + position.profit);
    }

    // 验证同一根 K 线中多个限价单会按挂单顺序撮合，并正确更新持仓均价。
    #[tokio::test]
    async fn matching_multiple_limit_orders_in_insertion_order() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let id1 = exchange.buy_limit(SYMBOL, 104.5, 1.0).await.unwrap();
        let id2 = exchange.buy_limit(SYMBOL, 105.5, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order1 = exchange.get_order(&id1).await.unwrap().unwrap();
        let order2 = exchange.get_order(&id2).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order1.status, Status::Filled);
        assert_eq!(order2.status, Status::Filled);
        assert_snap_eq(position.quantity, 2.0);
        assert_snap_eq(position.open_avg_price, 105.25);
    }

    // 验证触发市价单与普通市价单同Bar共存时，触发单遵循“两阶段撮合”。
    #[tokio::test]
    async fn matching_trigger_market_and_market_order_two_stage_behavior() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let market_id = exchange.buy(SYMBOL, 1.0).await.unwrap();
        let trigger_id = exchange
            .buy_trigger_market(SYMBOL, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let market_order = exchange.get_order(&market_id).await.unwrap().unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();

        assert_eq!(market_order.status, Status::Filled);
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发单立即执行，无 pending market order
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.quantity, 2.0);
        // 两个订单都在同一K线以开盘价105成交
        assert_snap_eq(position.open_avg_price, 105.0);
    }

    // 验证同Bar多条 reduce-only 平仓单竞争同一仓位时，后续订单会按剩余仓位处理。
    #[tokio::test]
    async fn matching_multiple_reduce_only_orders_share_single_position() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_id_1 = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        let close_id_2 = exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order_1 = exchange.get_order(&close_id_1).await.unwrap().unwrap();
        let close_order_2 = exchange.get_order(&close_id_2).await.unwrap().unwrap();

        assert_eq!(close_order_1.status, Status::Filled);
        assert_eq!(close_order_2.status, Status::Canceled);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证限价单冻结保证金金额精确匹配公式，撤单后余额完全归还。
    #[tokio::test]
    async fn margin_freeze_and_refund_for_limit_order_is_exact() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let qty = 1.23;
        let price = 90.0;
        let leverage = exchange.get_leverage(SYMBOL).await.unwrap() as f64;
        let expected_freeze = price * qty / leverage;

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy_limit(SYMBOL, price, qty).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_before - cash_after_place, expected_freeze);

        exchange.cancel_order(SYMBOL, &id).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_cancel, cash_before);
    }

    // 验证触发限价单在提交阶段不冻结，触发后转限价时才冻结保证金。
    #[tokio::test]
    async fn trigger_limit_freezes_margin_only_after_trigger() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange
            .buy_trigger_limit(SYMBOL, 105.0, 105.0, 1.0)
            .await
            .unwrap();
        let cash_after_submit = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_submit, cash_before);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 触发后立即执行，保证金和手续费都被扣除
        let cash_after_trigger = exchange.get_cash().await.unwrap();
        // 买单限价105≥开盘价105，以最高价106成交
        let fill_price = 106.0;
        let margin = fill_price * 1.0 / 10.0; // 10.6
        let fee = fill_price * 1.0 * 0.0002; // 0.0212 (maker fee)
        let total_cost = margin + fee; // 10.6212
        assert_snap_eq(cash_before - cash_after_trigger, total_cost);

        // 订单已执行，无法取消
        let position = exchange.get_position(SYMBOL).await.unwrap();
        assert!(position.is_some());
    }

    // 验证多条限价单撤单时余额返还等于冻结保证金总和。
    #[tokio::test]
    async fn cancel_all_refund_equals_total_frozen_margin_for_multiple_limits() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        exchange.buy_limit(SYMBOL, 80.0, 2.0).await.unwrap();

        let cash_after_place = exchange.get_cash().await.unwrap();
        let expected_freeze_total = 90.0 * 1.0 / 10.0 + 80.0 * 2.0 / 10.0;

        assert_snap_eq(cash_before - cash_after_place, expected_freeze_total);

        exchange.cancel_all_order(SYMBOL).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_cancel, cash_before);
    }

    // 验证市价单因保证金不足被拒绝时，不会错误扣减余额。
    #[tokio::test]
    async fn market_order_rejected_on_margin_shortage_keeps_cash_unchanged() {
        let exchange = LocalExchange::new(DataSource::new(default_metadata(), gen_kline_range()))
            .cash(1.0)
            .leverage(10);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after = exchange.get_cash().await.unwrap();
        let order = exchange.get_order(&id).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Rejected);
        assert_snap_eq(cash_after, cash_before);
    }

    // 验证 reduce-only 限价单挂单与撤单过程不会冻结或返还保证金。
    #[tokio::test]
    async fn reduce_only_limit_does_not_change_cash_on_place_or_cancel() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let id = exchange
            .sell_limit_reduce_only(SYMBOL, 120.0, 1.0)
            .await
            .unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_place, cash_before);

        exchange.cancel_order(SYMBOL, &id).await.unwrap();
        let cash_after_cancel = exchange.get_cash().await.unwrap();

        assert_snap_eq(cash_after_cancel, cash_before);
    }

    // 验证多仓被强平后，保证金损失、手续费与余额变化符合公式。
    #[tokio::test]
    async fn liquidation_long_pnl_margin_and_cash_match_formula() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 95.0, 96.0, 90.0, 92.0),
            ],
        );

        let md = default_metadata();
        let qty = 1.0;
        let open_price = 105.0;

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(SYMBOL)
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

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_snap_eq(history[0].profit, -init_margin);
        assert_snap_eq(history[0].fee, open_fee + liq_fee);
        assert_snap_eq(history[0].total_profit, -init_margin - open_fee - liq_fee);
        assert_snap_eq(cash_after, expected_cash);
    }

    // 验证空仓被强平后，保证金损失、手续费与余额变化符合公式。
    #[tokio::test]
    async fn liquidation_short_pnl_margin_and_cash_match_formula() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 115.0, 116.0, 114.0, 115.0),
            ],
        );

        let md = default_metadata();
        let qty = 1.0;
        let open_price = 105.0;

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.sell(SYMBOL, qty).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(SYMBOL)
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

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash_after = exchange.get_cash().await.unwrap();

        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_snap_eq(history[0].profit, -init_margin);
        assert_snap_eq(history[0].fee, open_fee + liq_fee);
        assert_snap_eq(history[0].total_profit, -init_margin - open_fee - liq_fee);
        assert_snap_eq(cash_after, expected_cash);
    }

    // 验证强平后权益应等于现金（无持仓未实现盈亏）。
    #[tokio::test]
    async fn equity_equals_cash_after_liquidation() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 95.0, 96.0, 90.0, 92.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let equity = exchange.get_equity().await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_snap_eq(equity, cash);
    }

    // 验证错误 symbol 的接口调用会返回错误，且不会污染当前账户状态。
    #[tokio::test]
    async fn symbol_mismatch_calls_do_not_contaminate_state() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

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
        assert_snap_eq(cash_after, cash_before);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证 range 边界可精确裁剪数据区间，并在末尾返回 None。
    #[tokio::test]
    async fn range_boundary_yields_expected_klines_then_none() {
        let exchange = test_exchange_with(default_metadata(), gen_kline_range()).range(2, 2);

        let first = exchange
            .next(SYMBOL, Level::Minute1)
            .await
            .unwrap()
            .unwrap();
        let second = exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert_eq!(first.time, 2);
        assert!(second.is_none());
    }

    // 验证触发单在 low/high 边界精确触发，略低于边界则保持未触发。
    #[tokio::test]
    async fn trigger_order_exact_low_boundary_triggers_but_below_does_not() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let exact_id = exchange
            .buy_trigger_market(SYMBOL, 104.0, 1.0)
            .await
            .unwrap();
        let below_id = exchange
            .buy_trigger_market(SYMBOL, 104.0 - 1e-12, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let exact = exchange.get_order(&exact_id).await.unwrap().unwrap();
        let below = exchange.get_order(&below_id).await.unwrap().unwrap();

        assert_eq!(exact.status, Status::Filled);
        assert_eq!(below.status, Status::Submitted);

        // 精确边界104.0触发后立即执行，以当前K线开盘价105成交
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position.quantity, 1.0);
        assert_snap_eq(position.open_avg_price, 104.0);
    }

    // 验证混合挂单按任意顺序撤销后，现金会回到撤单前基准值。
    #[tokio::test]
    async fn mixed_order_cancel_sequence_keeps_cash_invariant() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let limit_a = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let trigger = exchange
            .buy_trigger_limit(SYMBOL, 200.0, 90.0, 1.0)
            .await
            .unwrap();
        let limit_b = exchange.buy_limit(SYMBOL, 80.0, 2.0).await.unwrap();
        let reduce_only = exchange
            .sell_limit_reduce_only(SYMBOL, 120.0, 1.0)
            .await
            .unwrap();

        let cash_after_place = exchange.get_cash().await.unwrap();
        assert!(cash_after_place.snap_lt(cash_before, SNAP_TICK));

        exchange.cancel_order(SYMBOL, &reduce_only).await.unwrap();
        exchange.cancel_order(SYMBOL, &trigger).await.unwrap();
        exchange.cancel_order(SYMBOL, &limit_b).await.unwrap();
        exchange.cancel_order(SYMBOL, &limit_a).await.unwrap();

        let cash_after_cancel = exchange.get_cash().await.unwrap();
        assert_snap_eq(cash_after_cancel, cash_before);
        assert!(
            exchange
                .get_pending_order_list(SYMBOL)
                .await
                .unwrap()
                .is_empty()
        );
    }

    // 验证强平单在手续费不足时仍会执行平仓，避免仓位残留。
    #[tokio::test]
    async fn liquidation_executes_even_when_fee_cash_is_insufficient() {
        let exchange = LocalExchange::new(DataSource::new(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 95.0, 96.0, 90.0, 92.0),
            ],
        ))
        .cash(10.56)
        .leverage(10);

        let md = default_metadata();
        let open_price = 105.0;
        let qty = 1.0;

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let liq_price = exchange
            .get_pending_order_list(SYMBOL)
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

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
        assert!(history[0].is_liquidation());
        assert_snap_eq(history[0].profit, -10.5);
        assert!(history[0].fee.snap_gt(0.0, SNAP_TICK));
        assert_snap_eq(cash, expected_cash);
        assert!(cash.snap_lt(0.0, SNAP_TICK));
    }

    // 验证 metadata.min_size 非法（<=0）时，下单会被入口守卫拒绝。
    #[tokio::test]
    async fn place_order_rejects_when_metadata_min_size_is_zero() {
        let mut metadata = default_metadata();
        metadata.min_size = 0.0;

        let exchange = test_exchange_with(metadata, gen_kline_range());

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let result = exchange.buy(SYMBOL, 1.0).await.unwrap_err();

        assert!(result.to_string().contains("invalid metadata.min_size"));
    }

    // 验证同一根 K 线内连续下单时，订单 ID 唯一且挂单数量正确。
    #[tokio::test]
    async fn order_ids_are_unique_within_same_kline() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let id1 = exchange.buy_limit(SYMBOL, 90.0, 1.0).await.unwrap();
        let id2 = exchange.buy_limit(SYMBOL, 80.0, 1.0).await.unwrap();
        let id3 = exchange
            .sell_limit_reduce_only(SYMBOL, 120.0, 1.0)
            .await
            .unwrap();

        assert_ne!(id1, id2);
        assert_ne!(id1, id3);
        assert_ne!(id2, id3);

        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();
        assert_eq!(pending.len(), 3);
    }

    // 验证“调杠杆 + 调保证金”交替操作后，仓位与强平挂单价格始终同步。
    #[tokio::test]
    async fn leverage_and_margin_sequence_keeps_liquidation_in_sync() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_0 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_0 = position_0.liquidation_price;

        exchange.set_leverage(SYMBOL, 20).await.unwrap();
        let position_1 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_1 = position_1.liquidation_price;
        let liq_order_1 = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_1.snap_gt(liq_0, SNAP_TICK));
        assert_snap_eq(liq_order_1.price, liq_1);

        exchange.append_position_margin(SYMBOL, 2.0).await.unwrap();
        let position_2 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_2 = position_2.liquidation_price;
        let liq_order_2 = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_2.snap_lt(liq_1, SNAP_TICK));
        assert_snap_eq(liq_order_2.price, liq_2);

        exchange.append_position_margin(SYMBOL, -1.0).await.unwrap();
        let position_3 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_3 = position_3.liquidation_price;
        let liq_order_3 = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert!(liq_3.snap_gt(liq_2, SNAP_TICK));
        assert_snap_eq(liq_order_3.price, liq_3);
    }

    // 验证退化 K 线（开高低相同）下，限价与市价都按该唯一价格成交。
    #[tokio::test]
    async fn degenerate_kline_matches_limit_and_market_at_single_price() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 110.0, 110.0, 110.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let limit_id = exchange.buy_limit(SYMBOL, 110.0, 1.0).await.unwrap();
        let market_id = exchange.buy(SYMBOL, 1.0).await.unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let limit_order = exchange.get_order(&limit_id).await.unwrap().unwrap();
        let market_order = exchange.get_order(&market_id).await.unwrap().unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(limit_order.status, Status::Filled);
        assert_eq!(market_order.status, Status::Filled);
        assert_snap_eq(limit_order.avg_price, 110.0);
        assert_snap_eq(market_order.avg_price, 110.0);
        assert_snap_eq(position.quantity, 2.0);
        assert_snap_eq(position.open_avg_price, 110.0);
    }

    // 验证连续多次部分平仓会累计更新同一条历史仓位记录，直到最终全平。
    #[tokio::test]
    async fn sequential_partial_closes_accumulate_history_until_fully_closed() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 115.0, 116.0, 114.0, 115.0),
                gen_kline(5, 120.0, 121.0, 119.0, 120.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_after_first = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history_after_first = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_snap_eq(position_after_first.quantity, 1.5);
        assert_eq!(history_after_first.len(), 1);
        assert_snap_eq(history_after_first[0].close_quantity, 0.5);

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_after_second = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history_after_second = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_snap_eq(position_after_second.quantity, 1.0);
        assert_eq!(history_after_second.len(), 1);
        assert_snap_eq(history_after_second[0].close_quantity, 1.0);

        exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history_after_full_close = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history_after_full_close.len(), 1);
        assert_snap_eq(history_after_full_close[0].close_quantity, 2.0);
        assert_snap_eq(
            history_after_full_close[0].close_quantity,
            history_after_full_close[0].max_quantity,
        );
    }

    // 验证超量 reduce-only 平仓会被截断到当前仓位数量，且不会反向开仓。
    #[tokio::test]
    async fn reduce_only_oversized_close_is_clamped_to_position_quantity() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_id = exchange.sell_reduce_only(SYMBOL, 5.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let close_order = exchange.get_order(&close_id).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();

        assert_eq!(close_order.status, Status::Filled);
        assert_snap_eq(close_order.cumulative_quantity, 1.0);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
    }

    // 验证部分平仓后，仓位强平价与强平挂单价格保持同步一致。
    #[tokio::test]
    async fn partial_close_keeps_liquidation_order_price_synced() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liq_order = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        assert_eq!(position.side, Side::Buy);
        assert_eq!(liq_order.side, Side::Sell);
        assert_snap_eq(liq_order.price, position.liquidation_price);
    }

    // 验证多仓部分平仓后，现金/历史盈亏/手续费与剩余保证金严格符合公式。
    #[tokio::test]
    async fn partial_close_long_cash_and_history_match_formula() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        let md = default_metadata();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
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

        assert_snap_eq(position.quantity, 1.5);
        assert_snap_eq(position.margin, open_margin - close_margin);
        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].profit, close_profit);
        assert_snap_eq(history[0].fee, open_fee + close_fee);
        assert_snap_eq(history[0].total_profit, close_profit - open_fee - close_fee);
        assert_snap_eq(cash, expected_cash);
    }

    // 验证空仓部分平仓后，现金/历史盈亏/手续费与剩余保证金严格符合公式。
    #[tokio::test]
    async fn partial_close_short_cash_and_history_match_formula() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 100.0, 101.0, 99.0, 100.0),
            ],
        );

        let md = default_metadata();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
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

        assert_snap_eq(position.quantity, 1.5);
        assert_snap_eq(position.margin, open_margin - close_margin);
        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].profit, close_profit);
        assert_snap_eq(history[0].fee, open_fee + close_fee);
        assert_snap_eq(history[0].total_profit, close_profit - open_fee - close_fee);
        assert_snap_eq(cash, expected_cash);
    }

    // 验证已有多仓时，同向 reduce-only 订单会被取消且仓位不变。
    #[tokio::test]
    async fn reduce_only_same_side_with_position_is_canceled() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position_before = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        let id = exchange.buy_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        let position_after = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(order.status, Status::Canceled);
        assert_eq!(position_after.side, Side::Buy);
        assert_snap_eq(position_after.quantity, position_before.quantity);
        assert_snap_eq(
            position_after.open_avg_price,
            position_before.open_avg_price,
        );
    }

    // 验证连续部分平仓时，历史仓位的 close_avg_price 会更新为最近一次平仓价。
    #[tokio::test]
    async fn sequential_partial_close_updates_history_close_avg_price_to_latest() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 120.0, 121.0, 119.0, 120.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history_after_first = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history_after_first.len(), 1);
        assert_snap_eq(history_after_first[0].close_avg_price, 110.0);
        assert_snap_eq(history_after_first[0].close_quantity, 0.5);
        assert_snap_eq(history_after_first[0].profit, 2.5);
        assert_snap_eq(history_after_first[0].fee, 0.1325);
        assert_snap_eq(history_after_first[0].total_profit, 2.3675);

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history_after_second = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history_after_second.len(), 1);
        assert_snap_eq(history_after_second[0].close_avg_price, 120.0);
        assert_snap_eq(history_after_second[0].close_quantity, 1.0);
        assert_snap_eq(history_after_second[0].profit, 10.0);
        assert_snap_eq(history_after_second[0].fee, 0.1625);
        assert_snap_eq(history_after_second[0].total_profit, 9.8375);
    }

    // 验证“部分平仓 -> 加仓 -> 再部分平仓”时，close_quantity 表示累计已平仓量。
    #[tokio::test]
    async fn partial_close_quantity_tracks_cumulative_closed_quantity() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 115.0, 116.0, 114.0, 115.0),
                gen_kline(5, 120.0, 121.0, 119.0, 120.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_snap_eq(history[0].max_quantity, 2.5);
        assert_snap_eq(history[0].close_quantity, 1.0);
        assert_snap_eq(position.quantity, 2.0);
    }

    // 验证反向开仓会正确结算旧仓历史，并仅将超出部分作为新仓位。
    #[tokio::test]
    async fn reverse_order_splits_history_and_new_position_consistently() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        let md = default_metadata();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell(SYMBOL, 1.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let liq_order = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();

        let expected_profit = (110.0 - 105.0) * 1.0;
        let expected_fee = 105.0 * 1.0 * md.taker_fee + 110.0 * 1.0 * md.taker_fee;

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].side, Side::Buy);
        assert_snap_eq(history[0].close_quantity, 1.0);
        assert_snap_eq(history[0].max_quantity, 1.0);
        assert_snap_eq(history[0].profit, expected_profit);
        assert_snap_eq(history[0].fee, expected_fee);

        assert_eq!(position.side, Side::Sell);
        assert_snap_eq(position.quantity, 0.5);
        assert_snap_eq(position.open_avg_price, 110.0);
        assert_eq!(liq_order.side, Side::Buy);
        assert_snap_eq(liq_order.price, position.liquidation_price);
    }

    // 验证非 reduce-only 的对手方向“纯平仓”不会吞掉下单阶段冻结的保证金。
    #[tokio::test]
    async fn opposite_non_reduce_only_close_refunds_all_frozen_margin() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before_close = exchange.get_cash().await.unwrap();

        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after_close = exchange.get_cash().await.unwrap();

        // 从已开多仓直接卖出同等数量，现金变化应仅包含：
        // 平仓释放保证金 + 已实现盈亏 - 本次平仓手续费。
        let expected_delta = 10.5 + 5.0 - 110.0 * 1.0 * default_metadata().taker_fee;
        assert_snap_eq(cash_after_close - cash_before_close, expected_delta);
        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
    }

    // 验证仓位多次反转后会生成多条历史仓位，且各段方向/盈亏/手续费与资金结果一致。
    #[tokio::test]
    async fn multiple_reversals_create_multiple_history_positions_with_correct_values() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 100.0, 101.0, 99.0, 100.0),
                gen_kline(5, 120.0, 121.0, 119.0, 120.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 1) 开多 1（在 bar2 成交 105）
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 2) 卖 2（在 bar3 成交 110）：先平多 1，再反向开空 1
        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 3) 买 2（在 bar4 成交 100）：先平空 1，再反向开多 1
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 4) 卖出 reduce-only 1（在 bar5 成交 120）：平掉最后一段多仓
        exchange.sell_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history.len(), 3);

        // 第 1 段: 多仓 105 -> 110
        assert_eq!(history[0].side, Side::Buy);
        assert_snap_eq(history[0].open_avg_price, 105.0);
        assert_snap_eq(history[0].close_avg_price, 110.0);
        assert_snap_eq(history[0].max_quantity, 1.0);
        assert_snap_eq(history[0].close_quantity, 1.0);
        assert_snap_eq(history[0].profit, 5.0);
        assert_snap_eq(history[0].fee, 0.1075);
        assert_snap_eq(history[0].total_profit, 4.8925);

        // 第 2 段: 空仓 110 -> 100
        assert_eq!(history[1].side, Side::Sell);
        assert_snap_eq(history[1].open_avg_price, 110.0);
        assert_snap_eq(history[1].close_avg_price, 100.0);
        assert_snap_eq(history[1].max_quantity, 1.0);
        assert_snap_eq(history[1].close_quantity, 1.0);
        assert_snap_eq(history[1].profit, 10.0);
        assert_snap_eq(history[1].fee, 0.105);
        assert_snap_eq(history[1].total_profit, 9.895);

        // 第 3 段: 多仓 100 -> 120
        assert_eq!(history[2].side, Side::Buy);
        assert_snap_eq(history[2].open_avg_price, 100.0);
        assert_snap_eq(history[2].close_avg_price, 120.0);
        assert_snap_eq(history[2].max_quantity, 1.0);
        assert_snap_eq(history[2].close_quantity, 1.0);
        assert_snap_eq(history[2].profit, 20.0);
        assert_snap_eq(history[2].fee, 0.11);
        assert_snap_eq(history[2].total_profit, 19.89);

        // 总资金校验（反向开仓只保留新仓保证金，返还整单冻结与新仓保证金差额）
        assert_snap_eq(cash, 10034.6775);
    }

    // 验证空头起手的多次反转（空->多->空->平）会生成多条历史仓位，且各段数值正确。
    #[tokio::test]
    async fn multiple_reversals_from_short_side_create_correct_histories() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 100.0, 101.0, 99.0, 100.0),
                gen_kline(4, 120.0, 121.0, 119.0, 120.0),
                gen_kline(5, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 1) 开空 1（在 bar2 成交 105）
        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 2) 买 2（在 bar3 成交 100）：先平空 1，再反向开多 1
        exchange.buy(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 3) 卖 2（在 bar4 成交 120）：先平多 1，再反向开空 1
        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        // 4) 买入 reduce-only 1（在 bar5 成交 110）：平掉最后一段空仓
        exchange.buy_reduce_only(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let cash = exchange.get_cash().await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert_eq!(history.len(), 3);

        // 第 1 段: 空仓 105 -> 100
        assert_eq!(history[0].side, Side::Sell);
        assert_snap_eq(history[0].open_avg_price, 105.0);
        assert_snap_eq(history[0].close_avg_price, 100.0);
        assert_snap_eq(history[0].max_quantity, 1.0);
        assert_snap_eq(history[0].close_quantity, 1.0);
        assert_snap_eq(history[0].profit, 5.0);
        assert_snap_eq(history[0].fee, 0.1025);
        assert_snap_eq(history[0].total_profit, 4.8975);

        // 第 2 段: 多仓 100 -> 120
        assert_eq!(history[1].side, Side::Buy);
        assert_snap_eq(history[1].open_avg_price, 100.0);
        assert_snap_eq(history[1].close_avg_price, 120.0);
        assert_snap_eq(history[1].max_quantity, 1.0);
        assert_snap_eq(history[1].close_quantity, 1.0);
        assert_snap_eq(history[1].profit, 20.0);
        assert_snap_eq(history[1].fee, 0.11);
        assert_snap_eq(history[1].total_profit, 19.89);

        // 第 3 段: 空仓 120 -> 110
        assert_eq!(history[2].side, Side::Sell);
        assert_snap_eq(history[2].open_avg_price, 120.0);
        assert_snap_eq(history[2].close_avg_price, 110.0);
        assert_snap_eq(history[2].max_quantity, 1.0);
        assert_snap_eq(history[2].close_quantity, 1.0);
        assert_snap_eq(history[2].profit, 10.0);
        assert_snap_eq(history[2].fee, 0.115);
        assert_snap_eq(history[2].total_profit, 9.885);

        // 总资金校验（反向开仓只保留新仓保证金，返还整单冻结与新仓保证金差额）
        assert_snap_eq(cash, 10034.6725);
    }

    // 验证买入市价滑点超过上界时，成交价会被限制在 K 线 high。
    #[tokio::test]
    async fn market_buy_slippage_is_capped_by_kline_high() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.02);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 106.0);
    }

    // 验证卖出市价滑点超过下界时，成交价会被限制在 K 线 low。
    #[tokio::test]
    async fn market_sell_slippage_is_capped_by_kline_low() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.02);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 104.0);
    }

    // 验证限价穿价按开盘价成交时，会退还按下单价多冻结的保证金。
    #[tokio::test]
    async fn limit_cross_fill_refunds_excess_frozen_margin_when_fill_price_is_better() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 103.0, 105.0, 99.0, 100.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_before = exchange.get_cash().await.unwrap();
        let _id = exchange.buy_limit(SYMBOL, 110.0, 1.0).await.unwrap();
        let cash_after_place = exchange.get_cash().await.unwrap();

        // 下单时按委托价 110 冻结保证金：11.0
        assert_snap_eq(cash_before - cash_after_place, 11.0);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash_after_fill = exchange.get_cash().await.unwrap();
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        // 实际按最高价（对于买单最坏的） 105 成交，保证金应重算为 10.5，差额 0.5 返还。
        let expected_cash = cash_before - 10.5 - 105.0 * default_metadata().maker_fee;

        assert_snap_eq(position.open_avg_price, 105.0);
        assert_snap_eq(position.margin, 10.5);
        assert_snap_eq(cash_after_fill, expected_cash);
    }

    // 验证滑点在区间内时，买卖方向会按预期分别抬高/压低成交价。
    #[tokio::test]
    async fn market_slippage_applies_directionally_within_kline_range() {
        let buy_exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.005);

        buy_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let buy_id = buy_exchange.buy(SYMBOL, 1.0).await.unwrap();
        buy_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let buy_order = buy_exchange.get_order(&buy_id).await.unwrap().unwrap();
        assert_snap_eq(
            buy_order.avg_price,
            (105.0 * 1.005).snap_to_tick(default_metadata().tick_size),
        );

        let sell_exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.005);

        sell_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let sell_id = sell_exchange.sell(SYMBOL, 1.0).await.unwrap();
        sell_exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let sell_order = sell_exchange.get_order(&sell_id).await.unwrap().unwrap();
        assert_snap_eq(
            sell_order.avg_price,
            (105.0 * 0.995).snap_to_tick(default_metadata().tick_size),
        );
    }

    // 验证 slippage=0 时，市价成交行为与历史一致（按 open 成交）。
    #[tokio::test]
    async fn market_order_with_zero_slippage_fills_at_open() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.0);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 105.0);
    }

    // 验证限价单成交价不受 slippage 影响（仍遵循限价撮合规则）。
    #[tokio::test]
    async fn limit_order_fill_price_is_not_affected_by_slippage() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
            ],
        )
        .slippage(0.02);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let id = exchange.buy_limit(SYMBOL, 105.0, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let order = exchange.get_order(&id).await.unwrap().unwrap();
        assert_eq!(order.status, Status::Filled);
        assert_snap_eq(order.avg_price, 106.0);
    }

    // 验证触发市价单在二阶段成交时同样应用 slippage，并受 high/low 夹取。
    #[tokio::test]
    async fn trigger_market_fill_applies_slippage_with_kline_bounds() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        )
        .slippage(0.02);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let trigger_id = exchange
            .buy_trigger_market(SYMBOL, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，以当前K线开盘价加滑点成交，夹取到最高价
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        // 开盘价105 * (1 + 0.02滑点) = 107.1，夹取到最高价106
        assert_snap_eq(position.open_avg_price, 106.0);
    }

    // 验证触发市价卖单在二阶段成交时应用 slippage，并在低点边界处被夹取。
    #[tokio::test]
    async fn trigger_market_sell_fill_applies_slippage_with_kline_bounds() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 100.0, 101.0, 99.0, 100.0),
            ],
        )
        .slippage(0.05);

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let trigger_id = exchange
            .sell_trigger_market(SYMBOL, 105.0, 1.0)
            .await
            .unwrap();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let trigger_order = exchange.get_order(&trigger_id).await.unwrap().unwrap();
        assert_eq!(trigger_order.status, Status::Filled);

        // 触发后立即执行，以当前K线开盘价减滑点成交，夹取到最低价
        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position.side, Side::Sell);
        // 开盘价105 * (1 - 0.05滑点) = 99.75，夹取到最低价104
        assert_snap_eq(position.open_avg_price, 104.0);
    }

    // 验证反向开仓场景中，已追加保证金不会导致新仓位保证金出现负值。
    #[tokio::test]
    async fn reverse_with_appended_margin_keeps_new_margin_non_negative() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.append_position_margin(SYMBOL, 10.0).await.unwrap();
        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();

        assert_eq!(position.side, Side::Sell);
        assert_snap_eq(position.quantity, 1.0);
        assert!(position.margin.snap_gt(0.0, SNAP_TICK));
        assert_snap_eq(position.margin, 11.0);
    }

    // 验证 range 的 end_time 超过数据末尾时，仍会遍历到最后一根 K 线。
    #[tokio::test]
    async fn range_end_after_last_kline_still_iterates_to_end() {
        let exchange =
            test_exchange_with(default_metadata(), gen_kline_range()).range(2, 9999999999999);

        let first = exchange
            .next(SYMBOL, Level::Minute1)
            .await
            .unwrap()
            .unwrap();
        let second = exchange
            .next(SYMBOL, Level::Minute1)
            .await
            .unwrap()
            .unwrap();
        let third = exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        assert_eq!(first.time, 2);
        assert_eq!(second.time, 3);
        assert!(third.is_none());
    }

    // 验证空头加仓后的历史 max_quantity 统计按绝对仓位规模计算。
    #[tokio::test]
    async fn short_position_max_quantity_uses_absolute_exposure() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 100.0, 101.0, 99.0, 100.0),
                gen_kline(4, 95.0, 96.0, 94.0, 95.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.sell(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy_reduce_only(SYMBOL, 0.5).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let history = exchange.get_history_position_list(SYMBOL).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_snap_eq(history[0].max_quantity, 2.0);
    }

    // 验证 set_leverage 拒绝 0，避免后续除零产生无效资金计算。
    #[tokio::test]
    async fn set_leverage_rejects_zero() {
        let exchange = test_exchange();

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let result = exchange.set_leverage(SYMBOL, 0).await.unwrap_err();

        assert!(result.to_string().contains("greater than 0"));
    }

    // 压测：反向开仓 + 杠杆切换 + 保证金调整混合路径下，仓位与强平挂单始终同步且资金值有效。
    #[tokio::test]
    async fn stress_mixed_reverse_leverage_margin_path_keeps_invariants() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 110.0, 111.0, 109.0, 110.0),
                gen_kline(4, 95.0, 96.0, 94.0, 95.0),
                gen_kline(5, 120.0, 121.0, 119.0, 120.0),
                gen_kline(6, 100.0, 101.0, 99.0, 100.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        exchange.buy(SYMBOL, 1.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_open = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_open.margin, 105.0 * 1.0 / 10.0);

        exchange.append_position_margin(SYMBOL, 5.0).await.unwrap();
        let position_after_append = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_append.margin, 105.0 * 1.0 / 10.0 + 5.0);

        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_reverse = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position_after_reverse.side, Side::Sell);
        assert_snap_eq(position_after_reverse.quantity, 1.0);
        assert_snap_eq(position_after_reverse.margin, 110.0 * 1.0 / 10.0);

        exchange.set_leverage(SYMBOL, 20).await.unwrap();
        let position_after_leverage_20 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_leverage_20.margin, 110.0 * 1.0 / 20.0);

        exchange.append_position_margin(SYMBOL, 2.0).await.unwrap();
        let position_after_append_2 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_append_2.margin, 110.0 * 1.0 / 20.0 + 2.0);

        exchange.buy(SYMBOL, 0.6).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_partial_close = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position_after_partial_close.side, Side::Sell);
        assert_snap_eq(position_after_partial_close.quantity, 0.4);
        assert_snap_eq(position_after_partial_close.margin, 3.0);

        exchange.set_leverage(SYMBOL, 8).await.unwrap();
        let position_after_leverage_8 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_leverage_8.margin, 110.0 * 0.4 / 8.0);

        exchange.sell_reduce_only(SYMBOL, 0.3).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let position = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        let liquidation = exchange
            .get_pending_order_list(SYMBOL)
            .await
            .unwrap()
            .into_iter()
            .find(|v| v.kind == Kind::Liquidation)
            .unwrap();
        let cash = exchange.get_cash().await.unwrap();
        let equity = exchange.get_equity().await.unwrap();

        assert_snap_eq(position.margin, 110.0 * 0.4 / 8.0);
        assert_snap_eq(liquidation.price, position.liquidation_price);
        assert_eq!(liquidation.side, position.side.neg());
        assert!(cash.is_finite());
        assert!(equity.is_finite());
        assert!(equity.snap_gt(0.0, SNAP_TICK));
    }

    // 压测：多次交替调杠杆/调保证金/反向与平仓后，不变量持续成立且不会出现 NaN/Inf。
    #[tokio::test]
    async fn stress_repeated_mixed_operations_keep_exchange_state_sane() {
        let exchange = test_exchange_with(
            default_metadata(),
            vec![
                gen_kline(1, 100.0, 101.0, 99.0, 100.0),
                gen_kline(2, 105.0, 106.0, 104.0, 105.0),
                gen_kline(3, 95.0, 96.0, 94.0, 95.0),
                gen_kline(4, 115.0, 116.0, 114.0, 115.0),
                gen_kline(5, 90.0, 91.0, 89.0, 90.0),
                gen_kline(6, 120.0, 121.0, 119.0, 120.0),
                gen_kline(7, 100.0, 101.0, 99.0, 100.0),
            ],
        );

        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        exchange.buy(SYMBOL, 1.2).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_open = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_open.margin, 105.0 * 1.2 / 10.0);

        exchange.set_leverage(SYMBOL, 5).await.unwrap();
        let position_after_leverage_5 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_leverage_5.margin, 105.0 * 1.2 / 5.0);

        exchange.append_position_margin(SYMBOL, 3.0).await.unwrap();
        let position_after_append_3 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_append_3.margin, 105.0 * 1.2 / 5.0 + 3.0);

        exchange.sell(SYMBOL, 2.0).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_reverse = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position_after_reverse.side, Side::Sell);
        assert_snap_eq(position_after_reverse.quantity, 0.8);
        assert_snap_eq(position_after_reverse.margin, 95.0 * 0.8 / 5.0);

        exchange.set_leverage(SYMBOL, 15).await.unwrap();
        let position_after_leverage_15 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_leverage_15.margin, 95.0 * 0.8 / 15.0);

        exchange.append_position_margin(SYMBOL, 1.0).await.unwrap();
        let position_after_append_1 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_append_1.margin, 95.0 * 0.8 / 15.0 + 1.0);

        exchange.buy(SYMBOL, 1.1).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();
        let position_after_second_reverse = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_eq!(position_after_second_reverse.side, Side::Buy);
        assert_snap_eq(position_after_second_reverse.quantity, 0.3);
        assert_snap_eq(position_after_second_reverse.margin, 115.0 * 0.3 / 15.0);

        exchange.set_leverage(SYMBOL, 6).await.unwrap();
        let position_after_leverage_6 = exchange.get_position(SYMBOL).await.unwrap().unwrap();
        assert_snap_eq(position_after_leverage_6.margin, 115.0 * 0.3 / 6.0);

        exchange.close_all_position(SYMBOL).await.unwrap();
        exchange.next(SYMBOL, Level::Minute1).await.unwrap();

        let cash = exchange.get_cash().await.unwrap();
        let equity = exchange.get_equity().await.unwrap();
        let pending = exchange.get_pending_order_list(SYMBOL).await.unwrap();

        assert!(exchange.get_position(SYMBOL).await.unwrap().is_none());
        assert!(cash.is_finite());
        assert!(equity.is_finite());
        assert!(cash.snap_gt(0.0, SNAP_TICK));
        assert!(equity.snap_gt(0.0, SNAP_TICK));

        if let Some(position) = exchange.get_position(SYMBOL).await.unwrap() {
            let liq = pending
                .iter()
                .find(|v| v.kind == Kind::Liquidation)
                .unwrap();

            assert!(position.margin.snap_gt(0.0, SNAP_TICK));
            assert_eq!(liq.side, position.side.neg());
            assert_snap_eq(liq.price, position.liquidation_price);
        }
    }
}
