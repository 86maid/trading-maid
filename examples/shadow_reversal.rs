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

async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if cx.close.len() < 50 {
        return Ok(());
    }

    let atr_val = atr(cx.high, cx.low, cx.close, 14);
    let Some(atr) = atr_val else { return Ok(()) };

    if cx.get_position("BTCUSDT").await?.is_some() {
        return Ok(());
    }

    let body = (cx.open[0] - cx.close[0]).abs();

    // 长上影线做空：上影线 >= 实体 2 倍
    let upper_shadow = cx.high[0] - cx.open[0].max(cx.close[0]);
    if cx.close[0] < cx.open[0] && upper_shadow >= body * dec!(2) && body >= dec!(200) {
        let sl = round_to_tick(cx.high[0] + atr * dec!(0.5));
        let tp = round_to_tick(cx.low[0] - atr * dec!(3));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        return Ok(());
    }

    // 长下影线做多：下影线 >= 实体 2 倍
    let lower_shadow = cx.open[0].min(cx.close[0]) - cx.low[0];
    if cx.close[0] > cx.open[0] && lower_shadow >= body * dec!(2) && body >= dec!(200) {
        let sl = round_to_tick(cx.low[0] - atr * dec!(0.5));
        let tp = round_to_tick(cx.high[0] + atr * dec!(3));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());
}
