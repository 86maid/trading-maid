//! 趋势回调续势策略：EMA 排列定方向、ADX 定强度、突破前高定入场、ATR 移动止损定离场。
//!
//! 相对 `pullback_trend.rs` 的改进点：
//!
//! | 维度 | pullback_trend.rs | 本策略 |
//! |------|-------------------|--------|
//! | 趋势判定 | SMA50 vs SMA200 两条线 | EMA21 > EMA50 > EMA200 三线排列 + 收盘在 EMA200 之上 |
//! | 趋势强度 | 无 | Wilder ADX(14)：做多 >= 24，做空 >= 30 |
//! | 回调质量 | 与 SMA50 固定百分比带宽（0.99~1.005） | ATR 归一化，且不追高（收盘距 EMA21 <= 2 ATR） |
//! | 入场触发 | 阳线 + RSI 区间 | 收盘突破前一根高点（动量确认），避免下跌途中接刀 |
//! | 止损 | 最近 5 根极值（无风险约束） | 结构极值与 1.2 ATR 取宽者，且把单笔风险限制在 0.25~2.4 ATR |
//! | 止盈 | 固定 2R | 4R 限价止盈 + 1 ATR 移动止损（盈利 0.6R 后启动） |
//! | 出场 | 只能等 TP/SL | 另有持仓超时离场，移动止损只上移不下移 |
//!
//! 指标全部用带状态的缓存（`EMACache` / `RSICache` / 自实现 ADX），每根 K 线 O(1) 更新，
//! 不会像原版那样每根都重算整条序列。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);

/// ATR 与 ADX 的周期。
const ATR_N: usize = 14;
/// 结构极值回看的 K 线根数。
const SWING: usize = 6;
/// 三线 EMA 周期。
const EMA_FAST: usize = 21;
const EMA_MID: usize = 50;
const EMA_SLOW: usize = 200;

/// 做多/做空的趋势强度门槛。加密资产长期上行，做空需要更强的趋势证据。
const ADX_LONG: Decimal = dec!(24);
const ADX_SHORT: Decimal = dec!(30);
/// 入场时的 RSI 区间（做空取其镜像 32~62）。RSI 过低说明回调尚未结束。
const RSI_LOW: Decimal = dec!(38);
const RSI_HIGH: Decimal = dec!(68);

/// 收盘价距离快线的最大偏离，避免在已经走远的行情里追高。
const MAX_EXT_ATR: Decimal = dec!(2.0);
/// 止损的结构缓冲，以及止损距离的下限/上限（单位 ATR）。
const SL_BUFFER_ATR: Decimal = dec!(0.25);
const SL_MIN_ATR: Decimal = dec!(1.2);
const RISK_MIN_ATR: Decimal = dec!(0.25);
const RISK_MAX_ATR: Decimal = dec!(2.4);

/// 止盈 = 4R；盈利达到 0.6R 后启用 1 ATR 移动止损。
const TP_R: Decimal = dec!(4.0);
const TRAIL_START_R: Decimal = dec!(0.6);
const TRAIL_ATR: Decimal = dec!(1.0);
/// 持仓超过该根数仍未出场则市价离场，释放资金并避免长期占用。
const MAX_BAR_HELD: usize = 60;

/// 指标预热：EMA200 之外再留 12 根给 ADX 收敛。
const WARMUP: usize = EMA_SLOW + 12;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
///
/// 先按周期累加得到 TR / +DM / -DM 的初始平滑值，之后每次
/// `value = value - value / period + 新值`；DX 再做一次 Wilder 平滑得到 ADX。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_wilder: Option<Decimal>,
    plus_wilder: Option<Decimal>,
    minus_wilder: Option<Decimal>,
    dx_sum: Decimal,
    dx_count: usize,
    value: Option<Decimal>,
}

impl Adx {
    fn new(period: usize) -> Self {
        Adx {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            tr_sum: Decimal::ZERO,
            plus_sum: Decimal::ZERO,
            minus_sum: Decimal::ZERO,
            bar_count: 0,
            tr_wilder: None,
            plus_wilder: None,
            minus_wilder: None,
            dx_sum: Decimal::ZERO,
            dx_count: 0,
            value: None,
        }
    }

    fn update(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let Some(prev_high) = self.prev_high else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            return None;
        };

        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
        self.bar_count += 1;

        let period = Decimal::from(self.period);

        match self.tr_wilder {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_wilder = Some(self.tr_sum);
                self.plus_wilder = Some(self.plus_sum);
                self.minus_wilder = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_wilder.unwrap();
                let minus = self.minus_wilder.unwrap();

                self.tr_wilder = Some(value - value / period + tr);
                self.plus_wilder = Some(plus - plus / period + plus_dm);
                self.minus_wilder = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_wilder = self.tr_wilder.unwrap();

        if tr_wilder.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_wilder.unwrap() / tr_wilder * dec!(100);
        let minus_di = self.minus_wilder.unwrap() / tr_wilder * dec!(100);
        let di_sum = plus_di + minus_di;

        if di_sum.is_zero() {
            return self.value;
        }

        let dx = (plus_di - minus_di).abs() / di_sum * dec!(100);

        match self.value {
            None => {
                self.dx_sum += dx;
                self.dx_count += 1;

                if self.dx_count >= self.period {
                    self.value = Some(self.dx_sum / period);
                }
            }
            Some(prev) => {
                self.value = Some((prev * (period - dec!(1)) + dx) / period);
            }
        }

        self.value
    }
}

struct PullbackContinuation {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    bar_count: usize,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl PullbackContinuation {
    fn new() -> Self {
        PullbackContinuation {
            ema_fast: EMACache::new(EMA_FAST),
            ema_mid: EMACache::new(EMA_MID),
            ema_slow: EMACache::new(EMA_SLOW),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            bar_count: 0,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    /// 结构极值 + ATR 下限构成的止损价，`long` 为 false 时取镜像。
    fn stop_price(&self, long: bool, close: Decimal, atr: Decimal) -> Decimal {
        if long {
            let swing_low = self
                .low_buf
                .iter()
                .take(SWING)
                .copied()
                .fold(Decimal::MAX, Decimal::min);

            round_to_tick((swing_low - SL_BUFFER_ATR * atr).min(close - SL_MIN_ATR * atr))
        } else {
            let swing_high = self
                .high_buf
                .iter()
                .take(SWING)
                .copied()
                .fold(Decimal::MIN, Decimal::max);

            round_to_tick((swing_high + SL_BUFFER_ATR * atr).max(close + SL_MIN_ATR * atr))
        }
    }
}

#[async_trait(?Send)]
impl Strategy for PullbackContinuation {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        // ---- 指标状态每根都必须推进，预热期也不能漏 ----
        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);

        if self.high_buf.len() > SWING {
            self.high_buf.truncate(SWING);
        }

        if self.low_buf.len() > SWING {
            self.low_buf.truncate(SWING);
        }

        self.bar_count += 1;

        if self.bar_count < WARMUP {
            return Ok(());
        }

        let (Some(ema_fast), Some(ema_mid), Some(ema_slow)) = (ema_fast, ema_mid, ema_slow) else {
            return Ok(());
        };
        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };
        let Some(rsi) = self.rsi.value() else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // ---- 持仓管理：移动止损 + 超时离场 ----
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            let long = position.side == Side::Buy;
            let sign = if long { Decimal::ONE } else { -Decimal::ONE };
            let favorable = (close - entry_price) * sign;

            if favorable > self.best_favorable {
                self.best_favorable = favorable;
            }

            // 盈利达到 TRAIL_START_R 后，止损跟随“最佳收盘价 ± TRAIL_ATR * ATR”，只朝有利方向移动
            if self.best_favorable >= risk * TRAIL_START_R {
                let candidate = entry_price + sign * (self.best_favorable - TRAIL_ATR * atr);
                let current_stop = self.stop_price.unwrap_or(Decimal::ZERO);
                let improves = current_stop.is_zero()
                    || if long {
                        candidate > current_stop
                    } else {
                        candidate < current_stop
                    };

                if improves {
                    let new_stop = round_to_tick(candidate);

                    if new_stop != current_stop {
                        if let Some(id) = self.stop_order_id.clone() {
                            _ = cx.cancel_order(SYMBOL, id).await;
                        }

                        self.stop_order_id = if long {
                            cx.sell_trigger_market_reduce_only(SYMBOL, new_stop, position.quantity)
                                .await
                                .ok()
                        } else {
                            cx.buy_trigger_market_reduce_only(SYMBOL, new_stop, position.quantity)
                                .await
                                .ok()
                        };
                        self.stop_price = Some(new_stop);
                    }
                }
            }

            if self.bar_held >= MAX_BAR_HELD {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.reset_trade();
            }

            return Ok(());
        }

        self.reset_trade();

        // ---- 入场条件 ----
        let adx_value = adx.unwrap_or(Decimal::ZERO);
        let rsi_long_ok = rsi >= RSI_LOW && rsi <= RSI_HIGH;
        let rsi_short_ok =
            rsi >= Decimal::from(100) - RSI_HIGH && rsi <= Decimal::from(100) - RSI_LOW;

        let trend_long = close > ema_slow && ema_fast > ema_mid && ema_mid > ema_slow;
        let trend_short = close < ema_slow && ema_fast < ema_mid && ema_mid < ema_slow;
        let breakout_long = close > cx.high[1] && close > open;
        let breakout_short = close < cx.low[1] && close < open;

        if trend_long
            && adx_value >= ADX_LONG
            && rsi_long_ok
            && breakout_long
            && close - ema_fast <= MAX_EXT_ATR * atr
        {
            let sl = self.stop_price(true, close, atr);
            let risk = close - sl;

            if risk >= RISK_MIN_ATR * atr && risk <= RISK_MAX_ATR * atr {
                let tp = round_to_tick(close + TP_R * risk);

                if tp > close {
                    cx.cancel_all_order(SYMBOL).await?;
                    let (_, _, sl_result) = cx.buy_tp_sl(SYMBOL, tp, sl, QTY).await?;
                    self.entry_price = Some(close);
                    self.risk = Some(risk);
                    self.stop_price = Some(sl);
                    self.stop_order_id = sl_result.ok();

                    return Ok(());
                }
            }
        }

        if trend_short
            && adx_value >= ADX_SHORT
            && rsi_short_ok
            && breakout_short
            && ema_fast - close <= MAX_EXT_ATR * atr
        {
            let sl = self.stop_price(false, close, atr);
            let risk = sl - close;

            if risk >= RISK_MIN_ATR * atr && risk <= RISK_MAX_ATR * atr {
                let tp = round_to_tick(close - TP_R * risk);

                if tp < close && tp > Decimal::ZERO {
                    cx.cancel_all_order(SYMBOL).await?;
                    let (_, _, sl_result) = cx.sell_tp_sl(SYMBOL, tp, sl, QTY).await?;
                    self.entry_price = Some(close);
                    self.risk = Some(risk);
                    self.stop_price = Some(sl);
                    self.stop_order_id = sl_result.ok();
                }
            }
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径与 pullback_trend.rs 对齐：12 个月、4h。
    // 24/36/48 个月是同一份代码的稳健性检查（参数未按窗口调整）。
    for month_count in [12u32, 24, 36, 48] {
        let result = backtest(SYMBOL, month_count, PullbackContinuation::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
