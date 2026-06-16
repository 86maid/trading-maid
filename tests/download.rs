use trading_maid::util::{get_or_download_funding_rate, t2s};

// cargo test -r --test download -- --ignored
#[ignore]
#[tokio::test]
async fn main() {
    let fr = get_or_download_funding_rate("BTCUSDT", 0).await.unwrap();

    for v in fr {
        println!("{}: {}", t2s(v.time), v.funding_rate);
    }
}
