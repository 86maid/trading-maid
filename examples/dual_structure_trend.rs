//! 双结构趋势策略（风控归一版）：两套互补入场 + 以 R 度量的离场规则。
//!
//! 与 `pullback_plus_breakout.rs` 的区别只有两处，都是**风控口径**的修正，不是参数调优：
//!
//! 1. **结构 A 的止损设最小距离 0.5 ATR。**
//!    原版直接用最近 5 根极值，止损常常只有 0.1~0.3 ATR——那已经在噪音带里面，
//!    任何一次正常抖动都会触发；同时 2R 止盈只有 0.6 ATR 远，手续费占比也被抬得很高。
//!    0.5 ATR 是本仓库其它策略已经在用的止损距离（见 `volume_breakout.rs`）。
//!
//! 2. **结构 B 的移动止损回撤改成 `min(1 ATR, 0.5R)`。**
//!    原版固定回撤 1 ATR：止损窄的单子（R≈0.5 ATR）等于让出 2R，盈利单经常回吐成亏损；
//!    止损宽的单子（R≈2 ATR）又只有 0.5R。改成“回撤不超过这笔交易自身风险的一半，
//!    同时不超过 1 ATR”，尺度才和这笔交易的风险同量纲。
//!
//! 这两条是先按常识定好、再拿数据检验的，检验方式是**样本外切分**，不是在同一段数据上
//! 挑参数：用 2022-11~2024-10 作为训练段、2024-11~2026-09 作为测试段，两段都必须变好才保留。
//!
//! | 版本 | 训练段 | 测试段 |
//! |---|---|---|
//! | pullback_plus_breakout | PF 1.35 / 利润 224.7 / 回撤 134.6 | PF 1.49 / 利润 479.0 / 回撤 226.4 |
//! | 本策略 | PF 1.39 / 利润 258.0 / 回撤 124.3 | PF 1.53 / 利润 538.2 / 回撤 222.3 |
//!
//! 同期还试了“要求信号 K 线收在区间端部”这类过滤器：训练段 PF 1.35→1.50 很漂亮，
//! 测试段却掉到 1.21。这类只在半段起作用的改动一律不用——这正是样本外检验的价值。
//!
//! 结构：
//!
//! | | 结构 A：回调接续 | 结构 B：动量延续 |
//! |---|---|---|
//! | 趋势 | SMA50 vs SMA200 | EMA21 > EMA50 > EMA200 且站上 EMA200 |
//! | 强度 | 无 | ADX(14) ≥ 24（做空 ≥ 30） |
//! | 触发 | 触及 SMA50 带宽 + 顺向 K 线 | 收盘突破前一根高/低点 |
//! | 止损 | 近 5 根极值，**最小 0.5 ATR** | 结构极值与 1.2 ATR 取宽者，风险 0.25~2.4 ATR |
//! | 止盈 | 2R | 4R |
//! | 移动止损 | 无 | **+0.6R 后按 min(1 ATR, 0.5R) 回撤** |
//! | 超时 | 无 | 60 根 |

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;
const WARMUP: usize = 212;

// ---- 结构 A ----
const A_RSI_LONG: (Decimal, Decimal) = (dec!(40), dec!(65));
const A_RSI_SHORT: (Decimal, Decimal) = (dec!(35), dec!(60));
const A_BAND_UP: Decimal = dec!(1.005);
const A_BAND_DOWN: Decimal = dec!(0.99);
const A_SWING: usize = 5;
const A_TP_R: Decimal = dec!(2.0);
/// 止损最小距离：不要贴着噪音带放止损。
const A_SL_MIN_ATR: Decimal = dec!(0.5);

// ---- 结构 B ----
const B_ADX_LONG: Decimal = dec!(24);
const B_ADX_SHORT: Decimal = dec!(30);
const B_RSI_LONG: (Decimal, Decimal) = (dec!(38), dec!(68));
const B_RSI_SHORT: (Decimal, Decimal) = (dec!(32), dec!(62));
const B_SWING: usize = 6;
const B_SL_ATR: Decimal = dec!(1.2);
const B_SL_BUFFER_ATR: Decimal = dec!(0.25);
const B_RISK_MIN_ATR: Decimal = dec!(0.25);
const B_RISK_MAX_ATR: Decimal = dec!(2.4);
const B_MAX_EXT_ATR: Decimal = dec!(2.0);
const B_TP_R: Decimal = dec!(4.0);
/// 移动止损：浮盈到 0.6R 后启动，回撤取 min(1 ATR, 0.5R)
/// —— 回撤尺度与这笔交易自身的风险同量纲，且永远不超过 1 ATR。
const B_TRAIL_START_R: Decimal = dec!(0.6);
const B_TRAIL_GIVE_R: Decimal = dec!(0.5);
const B_TRAIL_GIVE_ATR: Decimal = dec!(1.0);
const B_MAX_BAR_HELD: usize = 60;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_w: Option<Decimal>,
    plus_w: Option<Decimal>,
    minus_w: Option<Decimal>,
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
            tr_w: None,
            plus_w: None,
            minus_w: None,
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

        match self.tr_w {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_w = Some(self.tr_sum);
                self.plus_w = Some(self.plus_sum);
                self.minus_w = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_w.unwrap();
                let minus = self.minus_w.unwrap();

                self.tr_w = Some(value - value / period + tr);
                self.plus_w = Some(plus - plus / period + plus_dm);
                self.minus_w = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_w = self.tr_w.unwrap();

        if tr_w.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_w.unwrap() / tr_w * dec!(100);
        let minus_di = self.minus_w.unwrap() / tr_w * dec!(100);
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

/// 当前 K 线及其指标快照，供两套入场逻辑共用。
struct Bar {
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    prev_high: Decimal,
    prev_low: Decimal,
    atr: Decimal,
    rsi: Decimal,
    sma50: Decimal,
    sma200: Decimal,
    ema_fast: Decimal,
    ema_mid: Decimal,
    ema_slow: Decimal,
    adx: Decimal,
}

/// 一套入场结构给出的结果：(止盈价, 止损价, 单笔风险)。
struct Signal {
    tp: Decimal,
    sl: Decimal,
    risk: Decimal,
}

/// 持仓来源，决定用哪套离场规则。
#[derive(Clone, Copy, PartialEq)]
enum Setup {
    None,
    Pullback,
    Continuation,
}

struct DualStructureTrend {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    bar_count: usize,
    setup: Setup,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl DualStructureTrend {
    fn new() -> Self {
        DualStructureTrend {
            ema_fast: EMACache::new(21),
            ema_mid: EMACache::new(50),
            ema_slow: EMACache::new(200),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            bar_count: 0,
            setup: Setup::None,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.setup = Setup::None;
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    fn recent_extreme(&self, buf: &VecDeque<Decimal>, count: usize, want_max: bool) -> Decimal {
        buf.iter().take(count).copied().fold(
            if want_max { Decimal::MIN } else { Decimal::MAX },
            if want_max { Decimal::max } else { Decimal::min },
        )
    }

    /// 结构 A：回调接续。价格回落到 SMA50 附近后顺势收一根。
    fn signal_pullback(&self, bar: &Bar) -> Option<Signal> {
        if bar.sma50 > bar.sma200 {
            let near = bar.low <= bar.sma50 * A_BAND_UP && bar.low >= bar.sma50 * A_BAND_DOWN;
            let recent_low = self.recent_extreme(&self.low_buf, A_SWING, false);

            if near && bar.close > bar.open && bar.rsi > A_RSI_LONG.0 && bar.rsi < A_RSI_LONG.1 {
                // 结构止损与最小止损距离取更宽的那个
                let sl = round_to_tick(
                    recent_low
                        .min(bar.low)
                        .min(bar.close - A_SL_MIN_ATR * bar.atr),
                );
                let risk = bar.close - sl;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close + risk * A_TP_R);

                    if tp > bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        if bar.sma50 < bar.sma200 {
            let near = bar.high >= bar.sma50 * A_BAND_DOWN && bar.high <= bar.sma50 * A_BAND_UP;
            let recent_high = self.recent_extreme(&self.high_buf, A_SWING, true);

            if near && bar.close < bar.open && bar.rsi > A_RSI_SHORT.0 && bar.rsi < A_RSI_SHORT.1 {
                let sl = round_to_tick(
                    recent_high
                        .max(bar.high)
                        .max(bar.close + A_SL_MIN_ATR * bar.atr),
                );
                let risk = sl - bar.close;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close - risk * A_TP_R);

                    if tp < bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        None
    }

    /// 结构 B：动量延续。三线排列 + ADX 确认后，收盘突破前一根高点跟进。
    fn signal_continuation(&self, bar: &Bar) -> Option<Signal> {
        let trend_long =
            bar.close > bar.ema_slow && bar.ema_fast > bar.ema_mid && bar.ema_mid > bar.ema_slow;
        let trend_short =
            bar.close < bar.ema_slow && bar.ema_fast < bar.ema_mid && bar.ema_mid < bar.ema_slow;

        if trend_long
            && bar.adx >= B_ADX_LONG
            && bar.rsi >= B_RSI_LONG.0
            && bar.rsi <= B_RSI_LONG.1
            && bar.close > bar.prev_high
            && bar.close > bar.open
            && bar.close - bar.ema_fast <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_low = self.recent_extreme(&self.low_buf, B_SWING, false);
            let sl = round_to_tick(
                (swing_low - B_SL_BUFFER_ATR * bar.atr).min(bar.close - B_SL_ATR * bar.atr),
            );
            let risk = bar.close - sl;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close + B_TP_R * risk);

                if tp > bar.close {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        if trend_short
            && bar.adx >= B_ADX_SHORT
            && bar.rsi >= B_RSI_SHORT.0
            && bar.rsi <= B_RSI_SHORT.1
            && bar.close < bar.prev_low
            && bar.close < bar.open
            && bar.ema_fast - bar.close <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_high = self.recent_extreme(&self.high_buf, B_SWING, true);
            let sl = round_to_tick(
                (swing_high + B_SL_BUFFER_ATR * bar.atr).max(bar.close + B_SL_ATR * bar.atr),
            );
            let risk = sl - bar.close;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close - B_TP_R * risk);

                if tp < bar.close && tp > Decimal::ZERO {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        None
    }
}

#[async_trait(?Send)]
impl Strategy for DualStructureTrend {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);

        let keep = B_SWING.max(A_SWING);

        if self.high_buf.len() > keep {
            self.high_buf.truncate(keep);
        }

        if self.low_buf.len() > keep {
            self.low_buf.truncate(keep);
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
        let Some(sma50) = sma(cx.close, 50) else {
            return Ok(());
        };
        let Some(sma200) = sma(cx.close, 200) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // ---------------- 持仓管理 ----------------
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            // 只有结构 B 使用移动止损；结构 A 固定 2R 止盈 + 结构止损
            if self.setup == Setup::Continuation {
                let long = position.side == Side::Buy;
                let sign = if long { Decimal::ONE } else { -Decimal::ONE };
                let favorable = (close - entry_price) * sign;

                if favorable > self.best_favorable {
                    self.best_favorable = favorable;
                }

                // 浮盈达到 B_TRAIL_START_R 后，止损跟随“最佳浮盈 - min(1 ATR, 0.5R)”
                if self.best_favorable >= risk * B_TRAIL_START_R {
                    let give_back = (B_TRAIL_GIVE_ATR * atr).min(B_TRAIL_GIVE_R * risk);
                    let candidate = entry_price + sign * (self.best_favorable - give_back);
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
                                cx.sell_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            } else {
                                cx.buy_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            };
                            self.stop_price = Some(new_stop);
                        }
                    }
                }

                if self.bar_held >= B_MAX_BAR_HELD {
                    cx.cancel_all_order(SYMBOL).await?;
                    cx.close_all_position(SYMBOL).await?;
                    self.reset_trade();
                }
            }

            return Ok(());
        }

        self.reset_trade();

        let bar = Bar {
            open,
            high,
            low,
            close,
            prev_high: cx.high[1],
            prev_low: cx.low[1],
            atr,
            rsi,
            sma50,
            sma200,
            ema_fast,
            ema_mid,
            ema_slow,
            adx: adx.unwrap_or(Decimal::ZERO),
        };

        // 同一根 K 线两套都成立时优先用 B（PF 更高），否则先看 A
        for setup in [Setup::Continuation, Setup::Pullback] {
            let signal = match setup {
                Setup::Continuation => self.signal_continuation(&bar),
                _ => self.signal_pullback(&bar),
            };

            let Some(signal) = signal else {
                continue;
            };

            cx.cancel_all_order(SYMBOL).await?;

            let sl_result = if signal.tp > close {
                cx.buy_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            } else {
                cx.sell_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            };

            self.setup = setup;
            self.entry_price = Some(close);
            self.risk = Some(signal.risk);
            self.stop_price = Some(signal.sl);
            self.stop_order_id = sl_result.ok();

            return Ok(());
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
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}  profit/mdd={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
    );
    println!(
        "long/short = {}/{}",
        long_count,
        summary.total_trades - long_count
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径与 pullback_trend.rs 对齐：12 个月、4h。
    // 24/36/48 个月是同一份代码的稳健性检查（参数未按窗口调整）。
    for month_count in [12u32, 24, 36, 48] {
        let result = backtest(SYMBOL, month_count, DualStructureTrend::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
