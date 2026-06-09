use trading_maid::prelude::*;

async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let body_size = (cx.open - cx.close).abs();
    let upper_shadow_size = (cx.high - cx.open).abs();
    let open_short_condition =
        cx.open > cx.close && upper_shadow_size >= body_size * 2.0 && body_size >= 300.0;

    if cx.get_position("BTCUSDT").await?.is_none() && open_short_condition {
        let take_profit_price = cx.open - upper_shadow_size;
        let stop_price = cx.open + upper_shadow_size;

        cx.cancel_all_order("BTCUSDT").await?;

        _ = cx
            .sell_tp_sl("BTCUSDT", take_profit_price, stop_price, 0.01)
            .await?;
    }

    Ok(())
}

#[ignore]
#[tokio::test]
async fn main() {
    let start = tokio::time::Instant::now();

    let path = get_or_download("BTCUSDT/1m", 12).await.unwrap();

    println!("load: {:?}", start.elapsed());

    let data_source_1m = DataSource::from_file_metadata(
        path,
        Metadata {
            symbol: "BTCUSDT".to_string(),
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

    let exchange = LocalExchange::new(data_source_1m.clone())
        .cash(10000.0)
        .leverage(10)
        .slippage(0.0);

    let mut engine = Engine::new(exchange.clone(), my_strategy);

    let start = tokio::time::Instant::now();

    if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
        println!("{:#?}", v);
    }

    println!("run: {:?}", start.elapsed());
}
