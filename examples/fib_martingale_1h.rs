//! 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（合约马丁格尔）——**1 小时级别**
//!
//! 与 ib_martingale.rs（4h / 0.382）规则完全相同，只把周期换成 1h。
//!
//! 规则完全按设计说明实现，参数不做扫描：
//!
//! 1. **拐点流动性 / 双顶双底**：用 3-3 分形找已确认的拐点。要求最近两个拐点高点
//!    （或低点）价格相近（≤ 1 ATR），构成双顶/双底——这些位置堆积流动性。
//! 2. **斐波那契坐标**：`0 = 顶`，`1 = 底`（做多时反过来 `0 = 底`，`1 = 顶`）。
//!    - 入场：价格回撤到 `0.382`
//!    - 止损基准：`0`（前高/前低），但按解套理论不用它硬止损
//!    - 止盈：`1.382`
//! 3. **真空区/缺口**：要求这段摆动里至少有一根实体 ≥ 1 ATR 的急促 K 线。
//!    这种价位没有像样的止损盘，反向砸起来也快，所以延续概率更高。
//! 4. **解套理论 + 马丁格尔**：入场后价格不但没回来，反而突破前高（做空方向即 `0` 位），
//!    按 `前高 + 0.1R` 加 0.01、`前高 + 0.2R` 再加 0.02，最多加两次（总 0.04）。
//!    加仓把均价拉到前高附近，等价格回抽前高时**保本/解套离场**；
//!    第三次加仓之后若继续逆行，用 `前高 + 1 ATR` 的硬止损兜底。
//!
//! 注意：马丁格尔让单笔最大仓位到 0.04（基础单的 4 倍），
//! 所以绝对利润不能和前面几个固定 0.01 手位的策略直接横向比较。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour1;
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;

/// 基础单与两次加仓的仓位
const BASE_QTY: Decimal = dec!(0.01);
const ADD1_QTY: Decimal = dec!(0.01);
const ADD2_QTY: Decimal = dec!(0.02);

/// 分形拐点的左右宽度
const PIVOT_SPAN: usize = 3;
/// 双顶/双底的价格相近度（ATR 倍数）
const DOUBLE_TOL_ATR: Decimal = dec!(1.0);
/// 摆动最小幅度（ATR 倍数），太小的摆动没有交易价值
const MIN_RANGE_ATR: Decimal = dec!(2.0);
/// 真空区判定：摆动内最大实体至少这么多 ATR
const VACUUM_BODY_ATR: Decimal = dec!(1.0);
/// 回撤/止盈/加仓的斐波那契位
const ENTRY_LEVEL: Decimal = dec!(0.382);
const TAKE_LEVEL: Decimal = dec!(1.382);
const ADD1_LEVEL: Decimal = dec!(-0.1);
const ADD2_LEVEL: Decimal = dec!(-0.2);
/// 三次加仓后的硬止损（ATR 倍数，位于 0 位之外）
const DISASTER_STOP_ATR: Decimal = dec!(1.0);

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// 一根 K 线的快照
#[derive(Clone)]
struct Candle {
    high: Decimal,
    low: Decimal,
    open: Decimal,
    close: Decimal,
}

impl Candle {
    fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }
}

/// 一个正在跟踪的斐波那契摆动
#[derive(Clone, Copy)]
struct Swing {
    /// 0 位价格（双顶的最高点 / 双底的最低点）
    origin: Decimal,
    /// 1 位价格（对面摆动极值）
    opposite: Decimal,
}

impl Swing {
    fn range(&self) -> Decimal {
        (self.origin - self.opposite).abs()
    }

    /// 按 0=顶、1=底 的坐标取价位；做多时 origin 是底，方向自动反过来。
    fn price_at(&self, level: Decimal) -> Decimal {
        self.origin + (self.opposite - self.origin) * level
    }
}

/// 一次持仓的马丁格尔阶梯
struct Ladder {
    long_side: bool,
    swing: Swing,
    unit_total: Decimal,
    add_count: usize,
    addition_list: [Decimal; 2],
    average_entry: Decimal,
}

struct FibMartingale {
    candle_list: VecDeque<Candle>,
    close_cache: VecDeque<Decimal>,
    bar_count: usize,
    ladder: Option<Ladder>,
    used_origin: Option<Decimal>,
}

impl FibMartingale {
    fn new() -> Self {
        FibMartingale {
            candle_list: VecDeque::new(),
            close_cache: VecDeque::new(),
            bar_count: 0,
            ladder: None,
            used_origin: None,
        }
    }

    /// 在 newest-first 的缓冲里找第 `skip` 个已确认拐点高点（返回索引）。
    fn find_pivot_high(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].high;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].high < value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].high < value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    fn find_pivot_low(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].low;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].low > value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].low > value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    /// 摆动区间内是否出现过急促实体（真空区）。
    fn has_vacuum(&self, from: usize, to: usize, atr: Decimal) -> bool {
        if from <= to {
            return false;
        }

        let threshold = VACUUM_BODY_ATR * atr;

        (to..from).any(|i| self.candle_list[i].body() >= threshold)
    }

    /// 找最近的“双顶”，返回摆动（0 = 顶，1 = 底）。
    fn find_double_top(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_high(0)?;
        let previous = self.find_pivot_high(1)?;
        let top = self.candle_list[last].high;
        let top_previous = self.candle_list[previous].high;

        if (top - top_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let bottom = (0..last)
            .map(|i| self.candle_list[i].low)
            .fold(Decimal::MAX, Decimal::min);
        let swing = Swing {
            origin: top,
            opposite: bottom,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }

    /// 找最近的“双底”，返回摆动（0 = 底，1 = 顶）。
    fn find_double_bottom(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_low(0)?;
        let previous = self.find_pivot_low(1)?;
        let bottom = self.candle_list[last].low;
        let bottom_previous = self.candle_list[previous].low;

        if (bottom - bottom_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let top = (0..last)
            .map(|i| self.candle_list[i].high)
            .fold(Decimal::MIN, Decimal::max);
        let swing = Swing {
            origin: bottom,
            opposite: top,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }
}

#[async_trait(?Send)]
impl Strategy for FibMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        self.candle_list.push_front(Candle {
            high,
            low,
            open,
            close,
        });

        if self.candle_list.len() > 80 {
            self.candle_list.truncate(80);
        }

        self.close_cache.push_front(close);

        if self.close_cache.len() > 200 {
            self.close_cache.truncate(200);
        }

        self.bar_count += 1;

        if self.bar_count < 60 {
            return Ok(());
        }

        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        let position = cx.get_position(SYMBOL).await?;

        // ---------------- 持仓管理：加仓 / 解套 / 硬止损 ----------------
        if position.is_some() {
            let Some(ladder) = self.ladder.as_mut() else {
                return Ok(());
            };

            let long_side = ladder.long_side;
            let origin = ladder.swing.origin;

            // 硬止损：0 位之外 1 ATR（由交易所的止损单兜底，这里只做状态清理）
            let disaster = if long_side {
                round_to_tick(origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(origin + DISASTER_STOP_ATR * atr)
            };
            let hit_disaster = if long_side {
                low <= disaster
            } else {
                high >= disaster
            };

            // 解套：已经加过仓，价格回抽 0 位（前高/前低）→ 保本离场
            let escape = ladder.add_count > 0
                && if long_side {
                    high >= origin
                } else {
                    low <= origin
                };

            // 加仓：价格突破 0 位一段距离
            let next_add = if ladder.add_count < ladder.addition_list.len() {
                Some(ladder.addition_list[ladder.add_count])
            } else {
                None
            };
            let need_add = next_add
                .map(|price| {
                    if long_side {
                        low <= price
                    } else {
                        high >= price
                    }
                })
                .unwrap_or(false);

            if need_add {
                let add_quantity = if ladder.add_count == 0 {
                    ADD1_QTY
                } else {
                    ADD2_QTY
                };
                let price = next_add.unwrap();

                // 用挂单价估算均价；实际成交价由撮合决定，方向一致即可
                let previous_notional = ladder.average_entry * ladder.unit_total;
                let new_total = ladder.unit_total + add_quantity;

                ladder.average_entry = (previous_notional + price * add_quantity) / new_total;
                ladder.unit_total = new_total;
                ladder.add_count += 1;

                let total = ladder.unit_total;
                let take = round_to_tick(ladder.swing.price_at(TAKE_LEVEL));

                // 必须先撤旧单，再下加仓单：cancel_all_order 会把刚挂出的市价单一起撤掉
                cx.cancel_all_order(SYMBOL).await?;

                if long_side {
                    _ = cx.buy(SYMBOL, add_quantity).await?;
                } else {
                    _ = cx.sell(SYMBOL, add_quantity).await?;
                }

                // 仓位变了，止盈止损要按新的总仓位重挂
                if long_side {
                    _ = cx.sell_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .sell_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                } else {
                    _ = cx.buy_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .buy_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                }

                return Ok(());
            }

            if hit_disaster || escape {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.ladder = None;
            }

            return Ok(());
        }

        self.ladder = None;

        // ---------------- 入场 ----------------
        let long_setup = self.find_double_bottom(atr);
        let short_setup = self.find_double_top(atr);

        // 先看空（双顶）再看多（双底）；同一根只做一笔
        for (long_side, swing) in [(false, short_setup), (true, long_setup)] {
            let Some(swing) = swing else {
                continue;
            };

            if self.used_origin == Some(swing.origin) {
                continue;
            }

            let entry_level = swing.price_at(ENTRY_LEVEL);
            let reached = if long_side {
                low <= entry_level
            } else {
                high >= entry_level
            };

            // 必须仍然在 0 位之内（还没被突破），否则这套结构已经失效
            let inside = if long_side {
                close > swing.origin
            } else {
                close < swing.origin
            };

            if !reached || !inside {
                continue;
            }

            let take = round_to_tick(swing.price_at(TAKE_LEVEL));
            let disaster = if long_side {
                round_to_tick(swing.origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(swing.origin + DISASTER_STOP_ATR * atr)
            };

            if (long_side && take <= close) || (!long_side && take >= close) {
                continue;
            }

            cx.cancel_all_order(SYMBOL).await?;

            if long_side {
                _ = cx.buy(SYMBOL, BASE_QTY).await?;
                _ = cx.sell_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .sell_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            } else {
                _ = cx.sell(SYMBOL, BASE_QTY).await?;
                _ = cx.buy_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .buy_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            }

            self.ladder = Some(Ladder {
                long_side,
                swing,
                unit_total: BASE_QTY,
                add_count: 0,
                addition_list: [
                    round_to_tick(swing.price_at(ADD1_LEVEL)),
                    round_to_tick(swing.price_at(ADD2_LEVEL)),
                ],
                average_entry: close,
            });
            self.used_origin = Some(swing.origin);

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
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let max_quantity = result
        .history_position
        .iter()
        .map(|v| v.max_quantity)
        .fold(Decimal::ZERO, Decimal::max);

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
        "total_profit={}  total_fee={}  max_drawdown={}  profit/mdd={}  max_ladder_qty={}  long/short={}/{}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        drawdown.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
        max_quantity,
        long_count,
        summary.total_trades - long_count,
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径：12 个月；24/36/48 个月做稳健性检查（参数未按窗口调整）。
    for month_count in [12u32, 24, 36, 48] {
        let result = backtest(SYMBOL, month_count, FibMartingale::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
