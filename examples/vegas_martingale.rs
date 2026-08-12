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

struct VegasMartingale {
    level: usize,
    ema12: EMACache,
    ema144: EMACache,
    ema169: EMACache,
}

impl VegasMartingale {
    fn new() -> Self {
        VegasMartingale {
            level: 0,
            ema12: EMACache::new(12),
            ema144: EMACache::new(144),
            ema169: EMACache::new(169),
        }
    }

    fn qty(&self) -> &str {
        match self.level {
            0 => "0.01",
            1 => "0.02",
            2 => "0.04",
            _ => "0.08",
        }
    }
}

#[async_trait(?Send)]
impl Strategy for VegasMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        if cx.close.len() < 200 {
            return Ok(());
        }

        let Some(ema12) = self.ema12.update(cx.close) else {
            return Ok(());
        };
        let Some(ema144) = self.ema144.update(cx.close) else {
            return Ok(());
        };
        let Some(ema169) = self.ema169.update(cx.close) else {
            return Ok(());
        };

        let rsi = rsi(cx.close, 14);
        let atr = atr(cx.high, cx.low, cx.close, 14);
        let (macd_line, _, histogram) = macd(cx.close, 12, 26, 9);

        let (Some(rsi), Some(atr), Some(_macd), Some(h)) = (rsi, atr, macd_line, histogram) else {
            return Ok(());
        };

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        cx.cancel_all_order("BTCUSDT").await?;

        let price = cx.close[0];

        let band_width = (ema144 - ema169).abs();
        let is_consolidating = band_width < atr * dec!(0.8);

        let trend_up = ema12 > ema144 && ema12 > ema169 && price > ema144 && price > ema169;
        let trend_down = ema12 < ema144 && ema12 < ema169 && price < ema144 && price < ema169;

        if is_consolidating {
            return Ok(());
        }

        if trend_up && h > dec!(0) && rsi > dec!(35) && rsi < dec!(55) {
            let recent_low = cx
                .low
                .iter()
                .take(5)
                .copied()
                .fold(Decimal::MAX, Decimal::min);
            let sl = round_to_tick(recent_low - atr * dec!(0.3));
            let tp = round_to_tick(price + (price - sl) * dec!(2));
            _ = cx.buy_tp_sl("BTCUSDT", tp, sl, self.qty()).await?;
            return Ok(());
        }

        if trend_down && h < dec!(0) && rsi > dec!(45) && rsi < dec!(65) {
            let recent_high = cx
                .high
                .iter()
                .take(5)
                .copied()
                .fold(Decimal::MIN, Decimal::max);
            let sl = round_to_tick(recent_high + atr * dec!(0.3));
            let tp = round_to_tick(price - (sl - price) * dec!(2));
            _ = cx.sell_tp_sl("BTCUSDT", tp, sl, self.qty()).await?;
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 48, VegasMartingale::new(), Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
