use crate::prelude::*;
use std::cell::RefCell;
use std::ops::{Deref, Index};

pub struct Context<'a> {
    pub time: &'a TimeSeries,
    pub open: &'a Series,
    pub high: &'a Series,
    pub low: &'a Series,
    pub close: &'a Series,
    pub volume: &'a Series,
    pub exchange: &'a ExchangeWrapper,
    pub series: Vec<(&'a str, &'a [Decimal])>,
    pub(crate) request_context: Option<&'a RequestContext<'a>>,
}

impl<'a> Deref for Context<'a> {
    type Target = ExchangeWrapper;

    fn deref(&self) -> &Self::Target {
        self.exchange
    }
}

impl<'a> Index<&str> for Context<'a> {
    type Output = Series;

    fn index(&self, key: &str) -> &Self::Output {
        self.series
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, data)| Series::new(data))
            .unwrap_or_else(|| Series::new(&[]))
    }
}

impl<'a> Context<'a> {
    /// Request another symbol's OHLCV context at the given level.
    ///
    /// - If `level` matches the strategy level or source level, pre-built data
    ///   is returned instantly.
    /// - For any other level, the data is **resampled from source klines** on
    ///   first access and cached for subsequent calls within the same strategy
    ///   invocation.
    ///
    /// Returns `None` if the symbol is unknown, or the level is invalid.
    pub fn request(&self, symbol: &str, level: Level) -> Option<Context<'a>> {
        let request_context = self.request_context?;
        let symbol_context = request_context
            .symbol
            .iter()
            .find(|(s, _)| s == symbol)
            .map(|(_, v)| v)?;

        let raw_context = if level == request_context.strategy_level {
            &symbol_context.strategy_context
        } else if level == request_context.source_level {
            &symbol_context.source_context
        } else {
            if !request_context.source_level.is_valid_sampling_target(level) {
                return None;
            }

            let mut level_kline = symbol_context.level_kline.borrow_mut();
            let index = level_kline.iter().position(|(l, _)| *l == level);

            if index.is_none() {
                let mut kline_buffer = KLineBuffer::new(0);

                kline_buffer.extend(resample(symbol_context.source_kline, level).ok()?);
                level_kline.push((level, kline_buffer));
            }

            let entry: &'a KLineBuffer = unsafe {
                std::mem::transmute(&level_kline[index.unwrap_or(level_kline.len() - 1)].1)
            };

            &KLineContext {
                time: entry.time.as_slice(),
                open: entry.open.as_slice(),
                high: entry.high.as_slice(),
                low: entry.low.as_slice(),
                close: entry.close.as_slice(),
                volume: entry.volume.as_slice(),
            }
        };

        Some(Context {
            time: TimeSeries::new(raw_context.time),
            open: Series::new(raw_context.open),
            high: Series::new(raw_context.high),
            low: Series::new(raw_context.low),
            close: Series::new(raw_context.close),
            volume: Series::new(raw_context.volume),
            exchange: self.exchange,
            series: request_context
                .series
                .iter()
                .filter(|((s, l, _), _)| s == symbol && *l == level)
                .map(|((_, _, name), aligned)| {
                    clip_series(aligned, name.as_str(), raw_context.time, level)
                })
                .collect(),
            request_context: Some(request_context),
        })
    }
}

pub(crate) struct KLineContext<'a> {
    pub time: &'a [u64],
    pub open: &'a [Decimal],
    pub high: &'a [Decimal],
    pub low: &'a [Decimal],
    pub close: &'a [Decimal],
    pub volume: &'a [Decimal],
}

pub(crate) struct SymbolContext<'a> {
    pub strategy_context: KLineContext<'a>,
    pub source_context: KLineContext<'a>,
    pub source_kline: &'a [KLine],
    pub level_kline: &'a RefCell<Vec<(Level, KLineBuffer)>>,
}

pub(crate) struct RequestContext<'a> {
    pub symbol: Vec<(String, SymbolContext<'a>)>,
    pub strategy_level: Level,
    pub source_level: Level,
    pub series: &'a Vec<((String, Level, String), AlignedSeries)>,
}

pub(crate) fn bar_offset_by_level(start: u64, end: u64, level: Level) -> usize {
    if level == Level::Month1 {
        // 月级别：逐月迭代，chrono 自动处理 28/29/30/31 天的差异
        let mut count = 0;
        let mut cur = start;
        while cur < end {
            if let Ok((_, next)) = get_time_range(cur, level) {
                cur = next;
                count += 1;
            } else {
                break;
            }
        }
        return count;
    }
    // 非月级别：间隔固定，直接做除法
    ((end - start) / level.interval_millis()) as usize
}

pub(crate) fn clip_series<'a>(
    aligned: &'a AlignedSeries,
    name: &'a str,
    time_slice: &[u64],
    level: Level,
) -> (&'a str, &'a [Decimal]) {
    // 前置检查：level 不匹配 / 时间列为空 → 返回空
    if aligned.level != level || time_slice.is_empty() {
        return (name, &[]);
    }

    // 步骤1：计算 OHLCV 数据覆盖的时间范围 [ctx_start, ctx_end)
    //
    // time_slice 是裸 &[u64]，未经过 Series::new 包装，按时间升序存储：
    //   time_slice[0]                     = 最早 bar 的开始时间
    //   time_slice[time_slice.len() - 1] = 最新 bar 的开始时间
    // ctx_start = 最早 bar 的开始时间（含）
    // ctx_end   = 最新 bar 的结束时间（不含）= 最新 bar 的 next
    let ctx_start = time_slice[0];
    let ctx_end = get_time_range(*time_slice.last().unwrap(), level)
        .ok()
        .map(|(_, next)| next)
        .unwrap_or(u64::MAX);

    // 步骤2：判断是否有交集
    //
    // 两个区间都是 [start, end) 左闭右开。
    // 有交集的充要条件：ctx_end > aligned.start && ctx_start < aligned.end
    if !(ctx_end > aligned.start && ctx_start < aligned.end) {
        return (name, &[]);
    }

    // 步骤3：只限制右边界，不限制左边界。
    //
    // 右边界必须对齐到 OHLCV 当前时间，不能看到"未来"的 bar。
    // 左边界不切——aligned.series 从 aligned.start 开始全保留，
    // 这样 cx[name][n] 可以回溯到交集之前的历史数据。
    //
    // 例如 aligned 覆盖 1月~6月，OHLCV 覆盖 3月~5月：
    //   cx[name][0] = 5月，cx[name][1] = 4月，...，一路拿到 1月
    let inter_end = ctx_end.min(aligned.end);
    let take = bar_offset_by_level(aligned.start, inter_end, level);

    // 步骤4：返回切片，从 aligned.series 开头取 take 个值
    (name, &aligned.series[..take])
}
