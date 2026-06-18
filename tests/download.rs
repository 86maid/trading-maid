use trading_maid::data::Level;
use trading_maid::util::{
    get_or_download_funding_rate, get_or_download_funding_rate_to_series, t2s,
};

// cargo test -r --test download funding_rate -- --ignored
#[ignore]
#[tokio::test]
async fn funding_rate() {
    let fr = get_or_download_funding_rate("BTCUSDT", 0).await.unwrap();

    println!("len: {}", fr.len());

    for v in &fr[..3] {
        println!("first: {}: {}", t2s(v.time), v.funding_rate);
    }

    for v in &fr[fr.len() - 3..] {
        println!("last: {}: {}", t2s(v.time), v.funding_rate);
    }
}

// cargo test -r --test download funding_rate_to_series -- --ignored
#[ignore]
#[tokio::test]
async fn funding_rate_to_series() {
    let aligned = get_or_download_funding_rate_to_series("BTCUSDT", 3, Level::Hour1)
        .await
        .unwrap();

    println!("len: {}", aligned.series.len());
    println!("first: {:?}", &aligned.series[..8.min(aligned.series.len())]);
    println!(
        "last: {:?}",
        &aligned.series[aligned.series.len().saturating_sub(8)..]
    );
}

// cargo test -r --test download engine_with_funding_rate -- --ignored
#[ignore]
#[tokio::test]
async fn engine_with_funding_rate() {
    use trading_maid::context::Context;
    use trading_maid::data::{DataSource, Metadata};
    use trading_maid::local_exchange::LocalExchange;
    use trading_maid::prelude::Engine;
    use trading_maid::util::get_or_download;

    let fr = get_or_download_funding_rate_to_series("BTCUSDT", 3, Level::Hour1)
        .await
        .unwrap();

    let data = DataSource::from_file_metadata(
        get_or_download("BTCUSDT/1m", 12).await.unwrap(),
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

    async fn strategy(cx: &Context<'_>) -> anyhow::Result<()> {
        if cx["funding_rate"] != [] {
            println!(
                "{}: current: {}, prev: {}",
                t2s(cx.time[0]),
                cx["funding_rate"][0],
                cx["funding_rate"][1]
            );
        }

        Ok(())
    }

    let mut engine = Engine::new(LocalExchange::new(data), strategy);

    engine.add_series("BTCUSDT", "funding_rate", fr);
    engine.run("BTCUSDT", Level::Hour1).await.unwrap();
}
