use crate::prelude::*;
use std::cell::RefCell;
use std::ops::{Deref, Index};

pub struct SeriesTable<'a>(pub(crate) Vec<(&'a str, &'a [Decimal])>);

impl<'a> Deref for SeriesTable<'a> {
    type Target = Vec<(&'a str, &'a [Decimal])>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> Index<&str> for SeriesTable<'a> {
    type Output = Series;

    fn index(&self, key: &str) -> &Self::Output {
        self.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, data)| Series::new(data))
            .unwrap_or_else(|| Series::new(&[]))
    }
}

pub struct Context<'a> {
    pub time: &'a TimeSeries,
    pub open: &'a Series,
    pub high: &'a Series,
    pub low: &'a Series,
    pub close: &'a Series,
    pub volume: &'a Series,
    pub exchange: &'a ExchangeWrapper,
    pub series: SeriesTable<'a>,
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
            series: SeriesTable(
                request_context
                    .series
                    .iter()
                    .filter(|((s, l, _), _)| s == symbol && *l == level)
                    .map(|((_, _, name), aligned)| {
                        clip_series(aligned, raw_context.time, name.as_str(), level)
                    })
                    .collect(),
            ),
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

pub(crate) fn clip_series<'a>(
    aligned_series: &'a AlignedSeries,
    strategy_time_slice: &[u64],
    name: &'a str,
    level: Level,
) -> (&'a str, &'a [Decimal]) {
    if aligned_series.level != level || strategy_time_slice.is_empty() {
        return (name, &[]);
    }

    let strategy_start = strategy_time_slice[0];
    let strategy_end = get_time_range(*strategy_time_slice.last().unwrap(), level)
        .ok()
        .map(|(_, next)| next)
        .unwrap_or(u64::MAX);

    if !(strategy_end > aligned_series.start && strategy_start < aligned_series.end) {
        return (name, &[]);
    }

    // 只限制右边界，不限制左边界
    // 右边界必须对齐到策略的当前时间，不能看到未来的 bar
    let end = strategy_end.min(aligned_series.end);
    let take = bar_offset_by_level(aligned_series.start, end, level);

    pub(crate) fn bar_offset_by_level(start: u64, end: u64, level: Level) -> usize {
        if level == Level::Month1 {
            let mut count = 0;
            let mut current = start;

            while current < end {
                if let Ok((_, next)) = get_time_range(current, level) {
                    current = next;
                    count += 1;
                } else {
                    break;
                }
            }

            return count;
        }

        ((end - start) / level.interval_millis()) as usize
    }

    (name, &aligned_series.series[..take])
}
