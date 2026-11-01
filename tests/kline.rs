//! Tier-1 tests: pure time / resampling / buffer logic. No engine, no exchange.

use trading_maid::prelude::*;

/// 2024-01-01T00:00:00Z
const T0: u64 = 1_704_067_200_000;
/// 2024-01-15T00:00:00Z
const JAN15: u64 = 1_705_276_800_000;
/// 2024-02-01T00:00:00Z
const FEB1: u64 = 1_706_745_600_000;
const MIN: u64 = 60_000;

fn kline(time: u64, close: Decimal) -> KLine {
    KLine {
        time,
        open: close,
        high: close,
        low: close,
        close,
        volume: dec!(1),
    }
}

#[test]
fn window_exposes_only_max_len() {
    let mut buffer = KLineBuffer::new(5);

    for i in 0..9u64 {
        buffer.push(kline(T0 + i * MIN, dec!(1)));
    }

    // 物理上还留着 9 根（攒到 2 倍才裁），但只暴露最后 5 根
    assert_eq!(buffer.time.len(), 9);
    assert_eq!(
        buffer.window().time,
        &(4..9).map(|i| T0 + i * MIN).collect::<Vec<_>>()[..]
    );
}

#[test]
fn trim_keeps_last_max_len() {
    let mut buffer = KLineBuffer::new(5);

    for i in 0..10u64 {
        buffer.push(kline(T0 + i * MIN, dec!(1)));
    }

    // 第 10 根触发裁剪，物理长度回到 5
    assert_eq!(buffer.time.len(), 5);
    assert_eq!(buffer.window().time.first(), Some(&(T0 + 5 * MIN)));
    assert_eq!(buffer.window().time.last(), Some(&(T0 + 9 * MIN)));
}

#[test]
fn pop_and_last() {
    let mut buffer = KLineBuffer::new(0);

    assert_eq!(buffer.pop(), None);
    assert_eq!(buffer.last(), None);

    buffer.push(kline(T0, dec!(1)));
    buffer.push(kline(T0 + MIN, dec!(2)));

    assert_eq!(buffer.last().unwrap().close, dec!(2));
    assert_eq!(buffer.pop().unwrap().close, dec!(2));
    assert_eq!(buffer.last().unwrap().close, dec!(1));
}

#[test]
fn resample_groups_bars_and_keeps_partial_tail() {
    // 7 根 1m：10:00 ~ 10:06，聚合成 5m 应该是「一组完整 + 一组未完成」
    let kline_list: Vec<KLine> = (0..7i32)
        .map(|i| KLine {
            time: T0 + i as u64 * MIN,
            open: Decimal::from(i),
            high: Decimal::from(i + 1),
            low: Decimal::from(i - 1),
            close: Decimal::from(i),
            volume: dec!(1),
        })
        .collect();

    let out = resample(&kline_list, Level::Minute5).unwrap();

    assert_eq!(out.len(), 2);

    assert_eq!(out[0].time, T0);
    assert_eq!(out[0].open, dec!(0));
    assert_eq!(out[0].high, dec!(5));
    assert_eq!(out[0].low, dec!(-1));
    assert_eq!(out[0].close, dec!(4));
    assert_eq!(out[0].volume, dec!(5));

    // 未走完的周期照样产出一根
    assert_eq!(out[1].time, T0 + 5 * MIN);
    assert_eq!(out[1].open, dec!(5));
    assert_eq!(out[1].high, dec!(7));
    assert_eq!(out[1].low, dec!(4));
    assert_eq!(out[1].close, dec!(6));
    assert_eq!(out[1].volume, dec!(2));
}

#[test]
fn time_range_aligns_to_level() {
    assert_eq!(
        get_time_range(T0 + 3 * MIN + 30_000, Level::Minute5).unwrap(),
        (T0, T0 + 5 * MIN)
    );

    assert_eq!(
        get_time_range(T0 + 3 * MIN, Level::Hour1).unwrap(),
        (T0, T0 + 60 * MIN)
    );

    // 已经是边界时原样返回
    assert_eq!(
        get_time_range(T0, Level::Minute5).unwrap(),
        (T0, T0 + 5 * MIN)
    );
}

#[test]
fn month1_follows_calendar() {
    assert_eq!(get_time_range(JAN15, Level::Month1).unwrap(), (T0, FEB1));
}

#[test]
fn last_time_of_period() {
    // 同一级别原样返回
    assert_eq!(
        get_last_time(JAN15, Level::Minute1, Level::Minute1).unwrap(),
        JAN15
    );

    // 1m 在 1h 周期里的最后一根
    assert_eq!(
        get_last_time(JAN15, Level::Minute1, Level::Hour1).unwrap(),
        JAN15 + 59 * MIN
    );

    // 1m 在月线周期里的最后一根：2024-01-31T23:59:00Z
    assert_eq!(
        get_last_time(JAN15, Level::Minute1, Level::Month1).unwrap(),
        FEB1 - MIN
    );
}

#[test]
fn invalid_sampling_target_is_rejected() {
    // 1h 不是 1m 的采样目标（反向）
    assert!(get_last_time(JAN15, Level::Hour1, Level::Minute1).is_err());
}
