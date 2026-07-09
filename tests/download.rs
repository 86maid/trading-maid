use trading_maid::prelude::*;

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
    use trading_maid::data::Level;
    use trading_maid::util::get_or_download_funding_rate_to_series;

    let aligned = get_or_download_funding_rate_to_series("BTCUSDT", 3, Level::Hour1)
        .await
        .unwrap();

    println!("len: {}", aligned.series.len());

    println!(
        "first: {:?}",
        &aligned.series[..8.min(aligned.series.len())]
    );

    println!(
        "last: {:?}",
        &aligned.series[aligned.series.len().saturating_sub(8)..]
    );
}

// cargo test -r --test download engine_with_funding_rate -- --ignored
#[ignore]
#[tokio::test]
async fn engine_with_funding_rate() {
    use trading_maid::data::{DataSource, Level, Metadata};
    use trading_maid::local_exchange::LocalExchange;
    use trading_maid::prelude::Engine;
    use trading_maid::util::{get_or_download, get_or_download_funding_rate_to_series, t2s};

    let fr = get_or_download_funding_rate_to_series("BTCUSDT", 1, Level::Hour1)
        .await
        .unwrap();

    let data = DataSource::from_file_metadata(
        get_or_download("BTCUSDT/1m", 1).await.unwrap(),
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

    let mut engine = Engine::new(LocalExchange::new(data).unwrap(), strategy);

    engine.add_series("BTCUSDT", "funding_rate", fr);
    engine.run("BTCUSDT", Level::Hour1).await.unwrap();
}

// cargo test -r --test download range -- --ignored
#[ignore]
#[tokio::test]
async fn range() {
    use trading_maid::data::{DataSource, Level, Metadata};
    use trading_maid::util::{get_or_download, get_or_download_funding_rate_to_series};

    let a = get_or_download_funding_rate("BTCUSDT", 2).await.unwrap();

    println!("funding_rate start: {}", t2s_utc(a[0].time));
    println!("funding_rate end: {}", t2s_utc(a.last().unwrap().time));

    let fr = get_or_download_funding_rate_to_series("BTCUSDT", 2, Level::Minute1)
        .await
        .unwrap();

    println!("funding_rate start: {}", t2s_utc(fr.start));
    println!("funding_rate end: {}", t2s_utc(fr.end));

    let data = DataSource::from_file_metadata(
        get_or_download("BTCUSDT/1m", 2).await.unwrap(),
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

    println!("kline start: {}", t2s_utc(data.data[0].time));
    println!("kline end: {}", t2s_utc(data.data.last().unwrap().time));
}

// cargo test -r --test download run -- --ignored
#[ignore]
#[tokio::test]
async fn run() {
    let fr = get_or_download_funding_rate_to_series("BTCUSDT", 3, Level::Hour4)
        .await
        .unwrap();

    let data = DataSource::from_file_metadata(
        get_or_download("BTCUSDT/1m", 1).await.unwrap(),
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

    struct RunTest {
        history: Vec<Decimal>,
    }

    #[async_trait(?Send)]
    impl Strategy for RunTest {
        async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
            if cx["funding_rate"] != [] {
                if !self.history.is_empty() && self.history.len() % 8 == 0 {
                    assert_eq!(
                        self.history[self.history.len() - 8..self.history.len()].to_vec(),
                        cx["funding_rate"][1..9].to_vec(),
                    );
                }

                self.history.push(cx["funding_rate"][0]);
            }

            Ok(())
        }
    }

    let mut engine = Engine::new(
        LocalExchange::new(data).unwrap(),
        RunTest {
            history: Vec::new(),
        },
    );

    engine.add_series("BTCUSDT", "funding_rate", fr);
    engine.run("BTCUSDT", Level::Hour4).await.unwrap();
}
