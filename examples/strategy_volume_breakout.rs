use std::collections::VecDeque;
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

fn donchian(high: &[Decimal], low: &[Decimal], period: usize) -> (Decimal, Decimal) {
    let upper = high
        .iter()
        .take(period)
        .copied()
        .fold(Decimal::MIN, Decimal::max);
    let lower = low
        .iter()
        .take(period)
        .copied()
        .fold(Decimal::MAX, Decimal::min);
    (upper, lower)
}

fn vol_ratio(volume: &[Decimal], period: usize) -> Option<Decimal> {
    if volume.len() < period + 1 {
        return None;
    }
    let avg_vol: Decimal =
        volume.iter().skip(1).take(period).sum::<Decimal>() / Decimal::from(period);
    if avg_vol.is_zero() {
        None
    } else {
        Some(volume[0] / avg_vol)
    }
}

struct Rma {
    period: usize,
    buf: VecDeque<Decimal>,
    value: Option<Decimal>,
}

impl Rma {
    fn new(period: usize) -> Self {
        Rma {
            period,
            buf: VecDeque::new(),
            value: None,
        }
    }
    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        self.buf.push_front(price);
        if self.buf.len() < self.period {
            let sum: Decimal = self.buf.iter().sum();
            self.value = Some(sum / Decimal::from(self.buf.len()));
            return self.value;
        }
        if self.buf.len() == self.period {
            let sum: Decimal = self.buf.iter().sum();
            self.value = Some(sum / Decimal::from(self.period));
            return self.value;
        }
        let alpha = dec!(1) / Decimal::from(self.period);
        if let Some(prev) = self.value {
            self.value = Some(alpha * price + (dec!(1) - alpha) * prev);
        }
        self.value
    }
}

struct VolumeBreakout {
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    vol_buf: VecDeque<Decimal>,
    rma_close: Rma,
    rma_vol: Rma,
}

impl VolumeBreakout {
    fn new() -> Self {
        VolumeBreakout {
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            vol_buf: VecDeque::new(),
            rma_close: Rma::new(20),
            rma_vol: Rma::new(20),
        }
    }
}

#[async_trait(?Send)]
impl Strategy for VolumeBreakout {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let high = cx.high[0];
        let low = cx.low[0];
        let vol = cx.volume[0];

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);
        self.vol_buf.push_front(vol);

        if self.high_buf.len() < 25 {
            self.rma_close.update(cx.close[0]);
            self.rma_vol.update(vol);
            return Ok(());
        }

        let ma_price = self.rma_close.update(cx.close[0]);
        let _ma_vol = self.rma_vol.update(vol);
        let Some(ma_price) = ma_price else {
            return Ok(());
        };

        let h: Vec<Decimal> = self.high_buf.iter().copied().collect();
        let l: Vec<Decimal> = self.low_buf.iter().copied().collect();
        let v: Vec<Decimal> = self.vol_buf.iter().copied().collect();

        let (dc_u, dc_l) = donchian(&h, &l, 20);
        let vr = vol_ratio(&v, 20);

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        let atr_val = atr(cx.high, cx.low, cx.close, 14);
        let Some(atr) = atr_val else { return Ok(()) };

        // 多头：唐奇安上轨突破 + 放量 1.3x + 价格在均线上方
        let has_vol = vr.map_or(false, |r| r > dec!(1.3));
        if cx.high[0] >= dc_u && has_vol && cx.close[0] > ma_price {
            let sl = round_to_tick(ma_price - atr * dec!(0.5));
            let tp = round_to_tick(cx.close[0] + atr * dec!(4));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
            return Ok(());
        }

        // 空头：唐奇安下轨跌破 + 放量 1.3x + 价格在均线下方
        if cx.low[0] <= dc_l && has_vol && cx.close[0] < ma_price {
            let sl = round_to_tick(ma_price + atr * dec!(0.5));
            let tp = round_to_tick(cx.close[0] - atr * dec!(4));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        }

        Ok(())
    }
}


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

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
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

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
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
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, VolumeBreakout::new(), Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
