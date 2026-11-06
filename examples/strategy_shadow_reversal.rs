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




const INITIAL_CASH: Decimal = dec!(1000000);
const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;


fn print_report(
    month_count: u32,
    symbol: &str,
    level: Level,
    result: &BacktestResult,
    drawdown: Decimal,
) {
    let summary = result.summarize();

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "total_trades={}  win_rate={}%  profit_loss_ratio={}  avg_profit={}  total_profit={}  max_drawdown={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        summary.avg_profit.round_dp(2),
        summary.total_profit.round_dp(2),
        drawdown.round_dp(2),
    );
    println!();
}

// backtest() 内部不给挂 hook，所以这里手工搭一遍同样的流程。
#[tokio::main]
async fn main() {
    let mut last = None;

    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let data_source = DataSource::from_file_metadata(
            get_or_download(format!("{}/1m", SYMBOL), month_count)
                .await
                .unwrap(),
            Metadata {
                symbol: SYMBOL.to_string(),
                level: Level::Minute1,
                min_size: "0.01".parse().unwrap(),
                min_notional: "0".parse().unwrap(),
                tick_size: "0.1".parse().unwrap(),
                maker_fee: "0.0002".parse().unwrap(),
                taker_fee: "0.0005".parse().unwrap(),
                maintenance: "0.004".parse().unwrap(),
            },
        )
        .unwrap();

        let exchange = LocalExchange::new(data_source.clone())
            .unwrap()
            .cash(INITIAL_CASH);

        let drawdown = DrawdownHook::new(INITIAL_CASH);

        let mut engine = Engine::new(exchange.clone(), 
my_strategy
);
        engine.hook(drawdown.clone());
        engine.run(SYMBOL, LEVEL).await.unwrap();

        let history_position = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let history_order = exchange.get_history_order_list(SYMBOL).await.unwrap();

        let result = BacktestResult {
            data_source,
            history_position,
            history_order,
        };

        print_report(month_count, SYMBOL, LEVEL, &result, drawdown.get());

        last = Some(result);
    }
    // 数据重采样到所有级别后可视化，只对最后一个窗口开一次
    if let Some(result) = last {
        result.resample_all_open_in_server().await.unwrap();
    }

}
