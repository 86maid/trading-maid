use anyhow::bail;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::cell::RefCell;
use trading_maid::context::Context;
use trading_maid::data::{AlignedSeries, DataSource, KLine, Level, Metadata};
use trading_maid::local_exchange::LocalExchange;
use trading_maid::prelude::Engine;
use trading_maid::strategy::Strategy;
use trading_maid::util::{get_or_download_funding_rate, s2t, t2s_utc};

// ============================================================
// 辅助函数
// ============================================================

/// 直接构造 AlignedSeries，不需要走 align_to_series。
///
/// `start_hour`..`end_hour` 按 1 小时粒度生成，值 = 小时序号，
/// 方便断言。
fn make_aligned_hourly(start_hour: i64, end_hour: i64) -> AlignedSeries {
    let start = s2t(&format!("2024/01/01 {:02}:00:00", start_hour));
    let end = s2t(&format!("2024/01/01 {:02}:00:00", end_hour));
    let series: Vec<Decimal> = (start_hour..end_hour).map(|i| Decimal::from(i)).collect();

    AlignedSeries {
        level: Level::Hour1,
        start,
        end,
        series,
    }
}

/// 构造 1 小时级别的 KLine 数据。
fn make_klines(start_hour: i64, end_hour: i64) -> Vec<KLine> {
    (start_hour..end_hour)
        .map(|h| {
            let time = s2t(&format!("2024/01/01 {:02}:00:00", h));
            KLine {
                time,
                open: Decimal::from(h),
                high: Decimal::from(h),
                low: Decimal::from(h),
                close: Decimal::from(h),
                volume: Decimal::ZERO,
            }
        })
        .collect()
}

/// 跑一次策略并取出最后一根 bar 上的 cx["fr"] 快照。
///
/// 策略在每根 bar 上都会被调用，这里只在时间 = `target` 时
/// 把 cx["fr"] 的值从 `[0]` 开始逐位收集到 `Vec`，遇到
/// `Decimal::MAX`（越界标记）就停止。
async fn run_and_capture(
    aligned: AlignedSeries,
    klines: Vec<KLine>,
    target_hour: i64,
) -> Vec<Decimal> {
    let target = s2t(&format!("2024/01/01 {:02}:00:00", target_hour));
    let captured: RefCell<Vec<Decimal>> = RefCell::new(Vec::new());

    struct CaptureStrategy<'a> {
        target: u64,
        captured: &'a RefCell<Vec<Decimal>>,
    }

    #[async_trait::async_trait(?Send)]
    impl Strategy for CaptureStrategy<'_> {
        async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
            if cx.time[0] == self.target {
                let mut values = Vec::new();
                for i in 0.. {
                    let v = cx["fr"][i];
                    if v == Decimal::MAX {
                        break;
                    }
                    values.push(v);
                }
                *self.captured.borrow_mut() = values;
            }
            Ok(())
        }
    }

    let ds = DataSource::new(
        Metadata {
            symbol: "BTCUSDT".to_string(),
            level: Level::Hour1,
            min_size: "0.01".parse().unwrap(),
            min_notional: "0".parse().unwrap(),
            tick_size: "0.1".parse().unwrap(),
            maker_fee: "0".parse().unwrap(),
            taker_fee: "0".parse().unwrap(),
            maintenance: "0".parse().unwrap(),
        },
        klines,
    );

    let mut engine = Engine::new(
        LocalExchange::new(ds),
        CaptureStrategy {
            target,
            captured: &captured,
        },
    );

    engine.add_series("BTCUSDT", "fr", aligned);
    engine.run("BTCUSDT", Level::Hour1).await.unwrap();

    captured.into_inner()
}

// ============================================================
// 测试：部分重叠 — OHLCV 是 series 的子集
// ============================================================

/// ```
/// aligned:  | 0──1──2──3──4──5──6──7──8──9 |  (0:00～10:00)
/// OHLCV:             | 4──5──6──7 |              (4:00～8:00)
/// ```
///
/// 期望：
/// - cx["fr"][0] = 7   （最新，对应 7:00 bar）
/// - cx["fr"][7] = 0   （回溯到 0:00，OHLCV 开始之前的数据）
/// - cx["fr"][8] = MAX （越界，series 只有 10 个值）
#[tokio::test]
async fn test_partial_overlap_backtrack_to_series_start() {
    let aligned = make_aligned_hourly(0, 10); // hours 0..10
    let klines = make_klines(4, 8); // hours 4..8

    let values = run_and_capture(aligned, klines, 7).await;

    assert_eq!(values.len(), 8, "应该在 7:00 bar 上拿到 8 个值（0..7）");
    assert_eq!(values[0], dec!(7)); // 最新
    assert_eq!(values[1], dec!(6));
    assert_eq!(values[2], dec!(5));
    assert_eq!(values[3], dec!(4));
    assert_eq!(values[4], dec!(3)); // ← 回溯到 OHLCV 开始之前
    assert_eq!(values[5], dec!(2));
    assert_eq!(values[6], dec!(1));
    assert_eq!(values[7], dec!(0)); // 最早
}

// ============================================================
// 测试：无交集 — series 完全在 OHLCV 左边
// ============================================================

/// ```
/// aligned:  | 0──1──2──3 |               (0:00～4:00)
/// OHLCV:                       | 6──7 |  (6:00～8:00)
/// ```
///
/// 期望：cx["fr"] == []（空）
#[tokio::test]
async fn test_no_overlap_series_before_ohlcv() {
    let aligned = make_aligned_hourly(0, 4); // hours 0..4
    let klines = make_klines(6, 8); // hours 6..8

    let values = run_and_capture(aligned, klines, 7).await;

    assert!(values.is_empty(), "无交集应该返回空");
}

// ============================================================
// 测试：无交集 — series 完全在 OHLCV 右边
// ============================================================

/// ```
/// aligned:                      | 6──7──8 |  (6:00～9:00)
/// OHLCV:      | 0──1──2──3 |               (0:00～4:00)
/// ```
///
/// 期望：cx["fr"] == []（空）
#[tokio::test]
async fn test_no_overlap_series_after_ohlcv() {
    let aligned = make_aligned_hourly(6, 9); // hours 6..9
    let klines = make_klines(0, 4); // hours 0..4

    let values = run_and_capture(aligned, klines, 3).await;

    assert!(values.is_empty(), "无交集应该返回空");
}

// ============================================================
// 测试：精确重叠
// ============================================================

/// ```
/// aligned:  | 4──5──6──7 |  (4:00～8:00)
/// OHLCV:    | 4──5──6──7 |  (4:00～8:00)
/// ```
///
/// 期望：4 个值，[0]=7, [3]=4, [4]=MAX
#[tokio::test]
async fn test_exact_overlap() {
    let aligned = make_aligned_hourly(4, 8); // hours 4..8
    let klines = make_klines(4, 8); // hours 4..8

    let values = run_and_capture(aligned, klines, 7).await;

    assert_eq!(values.len(), 4);
    assert_eq!(values[0], dec!(7));
    assert_eq!(values[1], dec!(6));
    assert_eq!(values[2], dec!(5));
    assert_eq!(values[3], dec!(4));
}

// ============================================================
// 测试：series 开始时间晚于 OHLCV 开始时间
// ============================================================

/// ```
/// aligned:             | 4──5──6──7──8──9 |  (4:00～10:00)
/// OHLCV:      | 1──2──3──4──5──6──7 |          (1:00～8:00)
/// ```
///
/// 期望：在 7:00 bar 上，能拿到 4..7 的值（4 个），
/// 不能回溯到 1～3（series 本身不覆盖）
#[tokio::test]
async fn test_series_starts_after_ohlcv_start() {
    let aligned = make_aligned_hourly(4, 10); // hours 4..10
    let klines = make_klines(1, 8); // hours 1..8

    let values = run_and_capture(aligned, klines, 7).await;

    assert_eq!(values.len(), 4, "series 只有 4..7，应该拿到 4 个值");
    assert_eq!(values[0], dec!(7));
    assert_eq!(values[1], dec!(6));
    assert_eq!(values[2], dec!(5));
    assert_eq!(values[3], dec!(4));
    // values[4] 不存在，因为 series 里没有 3
}

// ============================================================
// 测试：中间某根 bar 上，series 随 OHLCV 逐步增长
// ============================================================

/// ```
/// aligned:  | 0──1──2──3──4──5──6──7──8──9 |  (0:00～10:00)
/// OHLCV:    | 4──5──6──7 |                    (4:00～8:00)
/// ```
///
/// 在 5:00 bar（OHLCV 的第 2 根）上，只有 0..5 的值可用。
#[tokio::test]
async fn test_series_grows_with_ohlcv() {
    let aligned = make_aligned_hourly(0, 10); // hours 0..10
    let klines = make_klines(4, 8); // hours 4..8

    // 在 5:00 bar 上，ctx_end = 6:00，inter_end = 6:00，
    // take = bar_offset(0:00, 6:00) = 6，所以拿到 hours 0..6
    let values = run_and_capture(aligned, klines, 5).await;

    assert_eq!(values.len(), 6, "在 5:00 bar 上应该拿到 hours 0..5");
    assert_eq!(values[0], dec!(5)); // 最新
    assert_eq!(values[5], dec!(0)); // 最早
}

// ============================================================
// 原有的 download 测试（保留，标记 ignore）
// ============================================================

// cargo test -r --test download funding_rate -- --ignored
#[ignore]
#[tokio::test]
async fn funding_rate() {
    use trading_maid::util::{get_or_download_funding_rate, t2s};

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

    let mut engine = Engine::new(LocalExchange::new(data), strategy);

    engine.add_series("BTCUSDT", "funding_rate", fr);
    engine.run("BTCUSDT", Level::Hour1).await.unwrap();
}

// cargo test -r --test download range -- --ignored
#[ignore]
#[tokio::test]
async fn range() {
    use trading_maid::data::{DataSource, Level, Metadata};
    use trading_maid::local_exchange::LocalExchange;
    use trading_maid::prelude::Engine;
    use trading_maid::util::{get_or_download, get_or_download_funding_rate_to_series, t2s};

    let a = get_or_download_funding_rate("BTCUSDT", 2).await.unwrap();

    println!("fr start: {}", t2s_utc(a[0].time));
    println!("fr end: {}", t2s_utc(a.last().unwrap().time));

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

    println!("data start: {}", t2s_utc(data.data[0].time));
    println!("data end: {}", t2s_utc(data.data.last().unwrap().time));

    return;

    assert!(data.data[0].time == fr.start);
    assert!(data.data.last().unwrap().time == fr.end);

    async fn strategy(cx: &Context<'_>) -> anyhow::Result<()> {
        if cx["funding_rate"] != [] {
            println!(
                "{}: funding_rate[0]: {}, funding_rate[1..]: {:?}",
                t2s(cx.time[0]),
                cx["funding_rate"][0],
                cx["funding_rate"][1..9].to_vec(),
            );
        }

        Ok(())
    }

    let mut engine = Engine::new(LocalExchange::new(data), strategy);

    engine.add_series("BTCUSDT", "funding_rate", fr);
    engine.run("BTCUSDT", Level::Hour4).await.unwrap();
}
