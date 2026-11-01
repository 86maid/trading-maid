//! The `Context` passed to a strategy on every bar, exposing the current OHLCV
//! series, custom series and the exchange account methods.

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
    /// How many strategy-level bars went by without a call because the incoming bars were lost
    /// to a network interruption and had to be backfilled from the exchange.
    ///
    /// The backfilled bars are already inside `close`, so the series stays continuous and this
    /// counter is the only way to notice the jump. A stateful strategy can catch up by feeding
    /// `close[missed]`, `close[missed - 1]`, ..., `close[0]` in that order. `0` on a normal bar.
    pub missed: usize,
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
    /// - For any other level, the data is **resampled from source klines** and kept in sync
    ///   incrementally: each access only resamples the source k-lines that arrived since the
    ///   previous one, and the result is cached for the rest of the run.
    ///
    /// The newest bar of a level coarser than the source level may still be forming: its `close`
    /// tracks the latest source price and keeps changing until the period completes. Use
    /// `close[1]` for the last confirmed bar.
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
            &symbol_context.strategy_level_kline_context
        } else if level == request_context.source_level {
            &symbol_context.source_level_kline_context
        } else {
            if !request_context.source_level.is_valid_sampling_target(level) {
                return None;
            }

            let mut level_kline = symbol_context.level_kline_table.borrow_mut();

            let index = match level_kline.iter().position(|entry| entry.level == level) {
                Some(index) => index,
                None => {
                    level_kline.push(LevelKLine {
                        level,
                        buffer: KLineBuffer::new(request_context.max_size),
                        consumed: 0,
                        pending: false,
                    });

                    level_kline.len() - 1
                }
            };

            // 增量推进：只 resample 上次之后新增的 source 段
            level_kline[index]
                .refresh(symbol_context.source_kline_buffer)
                .ok()?;

            let entry: &'a KLineBuffer = unsafe { std::mem::transmute(&level_kline[index].buffer) };

            let window = entry.window();

            &KLineContext {
                time: window.time,
                open: window.open,
                high: window.high,
                low: window.low,
                close: window.close,
                volume: window.volume,
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
            missed: symbol_context.missed,
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
    pub strategy_level_kline_context: KLineContext<'a>,
    pub source_level_kline_context: KLineContext<'a>,
    pub source_kline_buffer: &'a [KLine],
    pub level_kline_table: &'a RefCell<Vec<LevelKLine>>,
    pub missed: usize,
}

pub(crate) struct LevelKLine {
    pub level: Level,
    pub buffer: KLineBuffer,
    pub consumed: usize,
    pub pending: bool,
}

impl LevelKLine {
    fn refresh(&mut self, source: &[KLine]) -> anyhow::Result<()> {
        // 没有新数据：[consumed, source.len()) 为空
        if source.len() <= self.consumed {
            return Ok(());
        }

        // source 的划分（全部左闭右开）：
        //   [0, consumed)                       已经折进 buffer 的完整 bar
        //   [consumed, forming_start_index)     本轮新走完的周期
        //   [forming_start_index, source.len()) 当前正在形成的周期

        // 最后一根 source bar 所属周期的起点，也就是当前周期的开头
        let forming_start = get_time_range(source[source.len() - 1].time, self.level)?.0;

        // 当前周期的第一根，即第一根 time >= forming_start 的 bar
        let forming_start_index = source[self.consumed..]
            .iter()
            .position(|kline| kline.time >= forming_start)
            .map(|offset| self.consumed + offset)
            .unwrap_or(source.len());

        // 有新走完的周期：区间 [consumed, forming_start_index) 非空
        if forming_start_index > self.consumed {
            // 上一轮那根半成品和将要生成的完整 bar 时间戳相同，先弹掉，否则会重复
            if self.pending {
                self.buffer.pop();
                self.pending = false;
            }

            // [consumed, forming_start_index) 全部是完整周期，可以放心追加
            let complete = resample(&source[self.consumed..forming_start_index], self.level)?;

            self.buffer.extend(complete);
            self.consumed = forming_start_index;
        }

        // 此时 consumed == forming_start_index，所以 [consumed, source.len()) 就是当前周期，
        // resample 只会产出一根
        let mut forming = resample(&source[self.consumed..], self.level)?;

        if let Some(last) = forming.pop() {
            // 和已缓存的尾部一模一样就什么都不写，保证对已经发出的引用零扰动
            if self.buffer.last() == Some(last) {
                return Ok(());
            }

            // 同一周期只保留一根：先弹掉旧的再推新的，等价于原地覆盖
            if self.pending {
                self.buffer.pop();
            }

            self.buffer.push(last);
            self.pending = true;
        }

        Ok(())
    }
}

pub(crate) struct RequestContext<'a> {
    pub symbol: Vec<(String, SymbolContext<'a>)>,
    pub strategy_level: Level,
    pub source_level: Level,
    pub series: &'a Vec<((String, Level, String), AlignedSeries)>,
    pub max_size: usize,
}

pub(crate) fn clip_series<'a>(
    aligned_series: &'a AlignedSeries,
    strategy_time_slice: &[u64],
    name: &'a str,
    strategy_level: Level,
) -> (&'a str, &'a [Decimal]) {
    if aligned_series.level != strategy_level || strategy_time_slice.is_empty() {
        return (name, &[]);
    }

    let strategy_start = strategy_time_slice[0];
    let strategy_end = get_time_range(*strategy_time_slice.last().unwrap(), strategy_level)
        .ok()
        .map(|(_, next)| next)
        .unwrap_or(u64::MAX);

    if !(strategy_end > aligned_series.start && strategy_start < aligned_series.end) {
        return (name, &[]);
    }

    // 只限制右边界，不限制左边界
    // 右边界必须对齐到策略的当前时间，不能看到未来的 bar
    let end = strategy_end.min(aligned_series.end);
    let take = bar_offset_by_level(aligned_series.start, end, strategy_level);

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
