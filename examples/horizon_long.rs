use trading_maid::prelude::*;

const WIN: usize = 2000;
const QTY: Decimal = dec!(0.05);
const SYMBOL: &str = "BTCUSDT";

// ---------- strategy configuration ----------
// momentum lookback (168 = 1 week of hours)
const MOM_BARS: usize = 168;
// fixed holding horizon (336 = 2 weeks)
const HOLD_BARS: usize = 336;
// disaster stop distance in ATR
const STOP_ATR: Decimal = dec!(6.0);
// bars to wait after a close (non-overlapping samples)
const MIN_GAP: usize = 24;
// classic TSMOM: hold only while momentum stays positive
const EXIT_ON_REVERSAL: bool = true;
const ATR_N: usize = 14;

// The 200-EMA "trend quality" filter was tested and REMOVED: it turned a profitable
// momentum strategy negative on both symbols (BTC 48m +1568 -> -177), because TSMOM
// works by catching moves before they are already extended above the long EMA.
// Kept as a constant only so the experiment is reproducible.
const REQUIRE_EMA200: bool = false;

fn atr_of(h: &[Decimal], l: &[Decimal], c: &[Decimal], period: usize) -> Option<Decimal> {
    let n = c.len();
    if period == 0 || n < period + 1 {
        return None;
    }
    let cnt = (n - 1).min(period);
    let mut s = Decimal::ZERO;
    for i in 0..cnt {
        let hi = h[i];
        let lo = l[i];
        let pc = c[i + 1];
        let r = if hi > lo { hi - lo } else { lo - hi };
        let u = if hi > pc { hi - pc } else { pc - hi };
        let d = if lo > pc { lo - pc } else { pc - lo };
        s += r.max(u).max(d);
    }
    Some(s / Decimal::from(cnt))
}

/// EMA over a newest-first slice.
fn ema_of(c: &[Decimal], period: usize) -> Option<Decimal> {
    let n = c.len();
    if period == 0 || n < period {
        return None;
    }
    let mut sum = Decimal::ZERO;
    for x in &c[n - period..] {
        sum += x;
    }
    let mut ema = sum / Decimal::from(period);
    for idx in (0..n - period).rev() {
        let v = c[idx];
        let alpha = dec!(2) / Decimal::from(period + 1);
        ema = v * alpha + ema * (dec!(1) - alpha);
    }
    Some(ema)
}

fn round_tick(p: Decimal) -> Decimal {
    let t = dec!(0.1);
    let r = (p / t).round_dp(0) * t;
    if r <= Decimal::ZERO { t } else { r }
}

struct TsmomLong {
    win: Vec<Decimal>,
    wih: Vec<Decimal>,
    wlo: Vec<Decimal>,
    in_position: bool,
    bars_held: usize,
    gap_left: usize,
    stop_price: Option<Decimal>,
    n_entries: usize,
    n_horizon_exit: usize,
    n_stop_exit: usize,
    n_reversal_exit: usize,
}

impl TsmomLong {
    fn new() -> Self {
        TsmomLong {
            win: Vec::new(),
            wih: Vec::new(),
            wlo: Vec::new(),
            in_position: false,
            bars_held: 0,
            gap_left: 0,
            stop_price: None,
            n_entries: 0,
            n_horizon_exit: 0,
            n_stop_exit: 0,
            n_reversal_exit: 0,
        }
    }
}

#[async_trait(?Send)]
impl Strategy for TsmomLong {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.win.insert(0, cx.close[0]);
        self.wih.insert(0, cx.high[0]);
        self.wlo.insert(0, cx.low[0]);
        if self.win.len() > WIN {
            for v in [&mut self.win, &mut self.wih, &mut self.wlo] {
                v.truncate(WIN);
            }
        }
        if self.win.len() <= MOM_BARS + 5 {
            return Ok(());
        }

        let pos = cx.get_position(SYMBOL).await?;
        let was_in = self.in_position;
        self.in_position = pos.is_some();
        if was_in && !self.in_position {
            self.gap_left = MIN_GAP;
        }

        // ---------- manage the open position ----------
        if pos.is_some() {
            self.bars_held += 1;
            let stop_hit = self.stop_price.map(|s| cx.close[0] <= s).unwrap_or(false);
            let horizon_done = self.bars_held >= HOLD_BARS;
            // signal-reversal exit: classic TSMOM holds only while momentum stays positive.
            // Require TWO consecutive closes below the momentum level: a single close can be
            // noise, and one-bar whipsaws were the dominant fee cost.
            let below =
                |k: usize| self.win.len() > MOM_BARS + k && self.win[k] < self.win[MOM_BARS + k];
            let mom_reversed = EXIT_ON_REVERSAL && below(0) && below(1);
            if stop_hit || horizon_done || mom_reversed {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                if horizon_done {
                    self.n_horizon_exit += 1;
                } else if stop_hit {
                    self.n_stop_exit += 1;
                } else {
                    self.n_reversal_exit += 1;
                }
                self.stop_price = None;
                self.bars_held = 0;
                self.in_position = false;
                self.gap_left = MIN_GAP;
            }
            return Ok(());
        }

        if self.gap_left > 0 {
            self.gap_left -= 1;
            return Ok(());
        }

        // ---------- entry: time-series momentum (long only) ----------
        let c0 = self.win[0];
        let past = self.win[MOM_BARS];
        // trend-quality filter: only act when price is above the 200-hour EMA, i.e. inside a
        // confirmed uptrend.  Momentum whipsaws badly in downtrends; this is a standard,
        // non-tuned filter (200 is the conventional long-period EMA).
        let ema200 = ema_of(&self.win, 200).unwrap_or(c0);
        let uptrend_ok = !REQUIRE_EMA200 || c0 > ema200;
        if c0 > past && uptrend_ok {
            let Some(atr) = atr_of(&self.wih, &self.wlo, &self.win, ATR_N) else {
                return Ok(());
            };
            let stop = round_tick(c0 - atr * STOP_ATR).max(dec!(1));
            cx.cancel_all_order(SYMBOL).await?;
            let _ = cx.buy(SYMBOL, QTY).await?;
            let _ = cx
                .sell_trigger_market_reduce_only(SYMBOL, stop, QTY)
                .await?;
            self.stop_price = Some(stop);
            self.bars_held = 0;
            self.in_position = true;
            self.n_entries += 1;
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let result = backtest(SYMBOL, 12, TsmomLong::new(), Level::Hour1)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
