//! Tests for how a data hole is reported through `Context.missed`.
//!
//! The local exchange cannot backfill anything — its data is a read-only snapshot — so a hole is
//! skipped instead. Every strategy-level bucket the hole touches is dropped whole: a bucket that
//! is missing some of its source bars has a wrong high/low/volume, yet its timestamp still lines
//! up, so handing it to the strategy would be an invisible corruption.

use std::{cell::RefCell, rc::Rc};

use trading_maid::prelude::*;

const MIN: u64 = 60_000;
const HOUR4: u64 = 240 * MIN;

/// 2023-01-01T00:00:00Z — aligned to a 4h boundary.
const T0: u64 = 1_672_531_200_000;

fn kline(time: u64) -> KLine {
    KLine {
        time,
        open: dec!(100),
        high: dec!(101),
        low: dec!(99),
        close: dec!(100),
        volume: Decimal::ONE,
    }
}

fn metadata() -> Metadata {
    Metadata {
        symbol: "BTCUSDT".to_string(),
        level: Level::Minute1,
        min_size: dec!(0.01),
        min_notional: dec!(0),
        tick_size: dec!(0.1),
        maker_fee: dec!(0.0002),
        taker_fee: dec!(0.0005),
        maintenance: dec!(0.004),
    }
}

/// 1m bars over `[start, end)`, leaving out the minutes inside `skip`.
fn bars(start: u64, end: u64, skip: Option<(u64, u64)>) -> Vec<KLine> {
    let mut result = Vec::new();
    let mut time = start;

    while time < end {
        let skipped = match skip {
            Some((skip_start, skip_end)) => time >= skip_start && time < skip_end,
            None => false,
        };

        if !skipped {
            result.push(kline(time));
        }

        time += MIN;
    }

    result
}

struct RecordingStrategy {
    missed: Rc<RefCell<Vec<Option<Missed>>>>,
    time: Rc<RefCell<Vec<u64>>>,
    volume: Rc<RefCell<Vec<Decimal>>>,
}

#[async_trait(?Send)]
impl Strategy for RecordingStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.missed.borrow_mut().push(cx.missed);
        self.time.borrow_mut().push(cx.time[0]);
        // 每根 1m 的 volume 都是 1，所以一根完整的 4h 桶应当正好是 240。
        self.volume.borrow_mut().push(cx.volume[0]);

        Ok(())
    }
}

/// Runs a 1m-source / 4h-strategy backtest and returns what the strategy saw.
async fn run(kline_list: Vec<KLine>) -> (Vec<Option<Missed>>, Vec<u64>, Vec<Decimal>) {
    let missed = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(RefCell::new(Vec::new()));
    let volume = Rc::new(RefCell::new(Vec::new()));

    let exchange = LocalExchange::new(vec![DataSource::new(metadata(), kline_list)])
        .unwrap()
        .cash(1_000_000)
        .leverage(1);

    let mut engine = Engine::new(
        exchange,
        RecordingStrategy {
            missed: missed.clone(),
            time: time.clone(),
            volume: volume.clone(),
        },
    );

    engine.run("BTCUSDT", Level::Hour4).await.unwrap();

    let missed = missed.borrow().clone();
    let time = time.borrow().clone();
    let volume = volume.borrow().clone();

    (missed, time, volume)
}

// 一个 4h 桶由 240 根 1m 组成，每根 volume 都是 1。
const FULL_BUCKET: Decimal = Decimal::from_parts(240, 0, 0, false, 0);

// 洞落在一个桶内部：这个桶整根丢掉，下一次调用的 missed 报 1 根缺失，时间跳两个周期。
#[tokio::test]
async fn hole_inside_a_bucket_drops_it_and_reports_missing_one() {
    // 第二个 4h 桶缺 40m-120m 共 80 分钟
    let kline_list = bars(
        T0,
        T0 + 12 * HOUR4 / 4,
        Some((T0 + HOUR4 + 40 * MIN, T0 + HOUR4 + 120 * MIN)),
    );

    let (missed, time, volume) = run(kline_list).await;

    assert_eq!(missed, vec![None, Some(Missed::Missing(1))]);
    assert_eq!(time, vec![T0, T0 + 2 * HOUR4]);
    // 交出去的两根都必须是完整桶，不能有任何一根带着缺失的 volume 混进来
    assert_eq!(volume, vec![FULL_BUCKET, FULL_BUCKET]);
}

// 洞跨两个桶：两个桶都丢掉，报 2 根缺失，时间跳三个周期。
#[tokio::test]
async fn hole_across_two_buckets_reports_missing_two() {
    // 第二个桶尾部缺 10 分钟，第三个桶开头缺 10 分钟
    let kline_list = bars(
        T0,
        T0 + 4 * HOUR4,
        Some((T0 + HOUR4 + 230 * MIN, T0 + 2 * HOUR4 + 10 * MIN)),
    );

    let (missed, time, volume) = run(kline_list).await;

    assert_eq!(missed, vec![None, Some(Missed::Missing(2))]);
    assert_eq!(time, vec![T0, T0 + 3 * HOUR4]);
    assert_eq!(volume, vec![FULL_BUCKET, FULL_BUCKET]);
}

// 没有洞时 missed 恒为 None，三根桶照常交付。
#[tokio::test]
async fn no_hole_reports_nothing() {
    let kline_list = bars(T0, T0 + 3 * HOUR4, None);

    let (missed, time, volume) = run(kline_list).await;

    assert_eq!(missed, vec![None, None, None]);
    assert_eq!(time, vec![T0, T0 + HOUR4, T0 + 2 * HOUR4]);
    assert_eq!(volume, vec![FULL_BUCKET, FULL_BUCKET, FULL_BUCKET]);
}

// 样本起点不在桶边界上，第一个桶天然只有半个桶——当前行为是照常交付。
//
// 这和「残桶丢弃」并不一致：那根桶的 volume 只有 120，high/low 也可能漏掉缺失时段的极值，
// 缺陷和残桶完全一样。这个用例把现状钉住，是否要改成丢弃另议。
#[tokio::test]
async fn sample_start_partial_bucket_is_delivered_as_is() {
    // 从第一个 4h 桶的第 2 小时开始，到第二个桶结束
    let kline_list = bars(T0 + 2 * 60 * MIN, T0 + 2 * HOUR4, None);

    let (missed, time, volume) = run(kline_list).await;

    assert_eq!(missed, vec![None, None]);
    assert_eq!(time, vec![T0, T0 + HOUR4]);
    // 第一根只有 120 分钟的数据
    assert_eq!(
        volume,
        vec![Decimal::from_parts(120, 0, 0, false, 0), FULL_BUCKET]
    );
}
