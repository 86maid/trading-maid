use std::cell::RefCell;
use std::ops::{Deref, Index};

use rust_decimal::Decimal;

use crate::data::{AlignedSeries, KLine, Level};
use crate::util::get_time_range;
use crate::exchange::*;
use crate::series::*;

/// Per-symbol OHLCV slices borrowed from a backing columnar store
/// ([`KLineBuffer`](crate::data::KLineBuffer) or [`LevelCacheEntry`]).
pub(crate) struct SymbolSlices<'a> {
    pub time: &'a [u64],
    pub open: &'a [Decimal],
    pub high: &'a [Decimal],
    pub low: &'a [Decimal],
    pub close: &'a [Decimal],
    pub volume: &'a [Decimal],
}

/// Owned columnar OHLCV data for a single level, without a const-generic capacity.
///
/// Used as the cache value in [`SymbolBuffer::level_cache`] so that
/// [`MultiSymbolData`] can hold a type-erased pointer to the cache map.
#[derive(Default)]
pub(crate) struct LevelCacheEntry {
    pub time: Vec<u64>,
    pub open: Vec<Decimal>,
    pub high: Vec<Decimal>,
    pub low: Vec<Decimal>,
    pub close: Vec<Decimal>,
    pub volume: Vec<Decimal>,
}

impl LevelCacheEntry {
    fn extend_from_klines(&mut self, klines: &[KLine]) {
        for k in klines {
            self.time.push(k.time);
            self.open.push(k.open);
            self.high.push(k.high);
            self.low.push(k.low);
            self.close.push(k.close);
            self.volume.push(k.volume);
        }
    }
}

/// Pre-built slices and on-demand cache for one symbol in a multi-symbol run.
pub(crate) struct MultiSymbolData<'a> {
    /// Strategy-level OHLCV slices (pre-built, matches the `level` passed to
    /// [`Engine::run_multi`](crate::engine::Engine::run_multi)).
    pub strategy: SymbolSlices<'a>,
    /// Source-level OHLCV slices (pre-built from raw exchange data).
    pub source: SymbolSlices<'a>,
    /// Source klines kept for on-demand resampling to levels other than source
    /// and strategy.
    pub source_klines: &'a [KLine],
    /// Reference to the owning [`SymbolBuffer`]'s level cache.
    ///
    /// `RefCell` provides interior mutability because [`Context::request`]
    /// takes `&self` but needs to mutate the cache on first access.
    pub level_cache: &'a RefCell<Vec<(Level, LevelCacheEntry)>>,
}

/// Holds data for all symbols in a multi-symbol run.
///
/// Built by [`Engine::run_multi`](crate::engine::Engine::run_multi) before each
/// strategy invocation and exposed via [`Context::multi`].
pub(crate) struct MultiSlices<'a> {
    pub symbols: Vec<(String, MultiSymbolData<'a>)>,
    pub exchange: &'a ExchangeWrapper,
    pub strategy_level: Level,
    pub source_level: Level,
    pub series: &'a Vec<((String, Level, String), AlignedSeries)>,
}

pub struct Context<'a> {
    pub time: &'a TimeSeries,
    pub open: &'a Series,
    pub high: &'a Series,
    pub low: &'a Series,
    pub close: &'a Series,
    pub volume: &'a Series,
    pub exchange: &'a ExchangeWrapper,
    pub series: Vec<(&'a str, &'a [Decimal])>,
    /// Set when running multi-symbol; enables [`Context::request`].
    pub(crate) multi: Option<&'a MultiSlices<'a>>,
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
        static EMPTY: &[Decimal] = &[];
        self.series
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, data)| Series::new(data))
            .unwrap_or_else(|| Series::new(EMPTY))
    }
}

/// Count the number of [`Level`] bars between two timestamps
/// (`start` ≤ `end`).
///
/// For calendar-aware levels (currently [`Month1`]) this iterates via
/// [`get_time_range`]; for all other levels the arithmetic fast-path
/// `(end - start) / interval_millis()` is used.
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

/// Slice an [`AlignedSeries`] to the time-intersection with the given OHLCV
/// time column.
///
/// - `slices_time` is the pre-built time column of the OHLCV data
///   (chronological order: `last()` = earliest bar, `first()` = latest bar).
/// - `level` must match `aligned.level`.
///
/// Returns `(name, &[Decimal])`.  If there is no time overlap, the slice is
/// empty.
pub(crate) fn slice_aligned_series<'a>(
    aligned: &'a AlignedSeries,
    name: &'a str,
    slices_time: &[u64],
    level: Level,
) -> (&'a str, &'a [Decimal]) {
    // 前置检查：level 不匹配 / 时间列为空 → 返回空
    if aligned.level != level || slices_time.is_empty() {
        return (name, &[]);
    }

    // 步骤1：计算 OHLCV 数据覆盖的时间范围 [ctx_start, ctx_end)
    //
    // slices_time 按时间升序存储：
    //   slices_time[0]                     = 最新 bar 的开始时间
    //   slices_time[slices_time.len() - 1] = 最早 bar 的开始时间
    // ctx_start = 最早 bar 的开始时间（含）
    // ctx_end   = 最新 bar 的结束时间（不含）= 最新 bar 的 next
    let ctx_start = slices_time[slices_time.len().saturating_sub(1)];
    let ctx_end = get_time_range(slices_time[0], level).ok().map(|(_, next)| next).unwrap_or(u64::MAX);

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

impl<'a> Context<'a> {
    /// Request another symbol's OHLCV context at the given level.
    ///
    /// - If `level` matches the strategy level or source level, pre-built data
    ///   is returned instantly.
    /// - For any other level, the data is **resampled from source klines** on
    ///   first access and cached for subsequent calls within the same strategy
    ///   invocation.
    ///
    /// Returns `None` if this context was created by [`Engine::run`](crate::engine::Engine::run)
    /// (single-symbol), the symbol is unknown, or the level is invalid.
    pub fn request(&self, symbol: &str, level: Level) -> Option<Context<'a>> {
        // 只在多标的模式下可用
        let multi = self.multi?;
        // 查找目标标的的数据
        let sym = multi.symbols.iter().find(|(s, _)| s == symbol).map(|(_, v)| v)?;

        // 步骤1：获取 OHLCV 切片（三种来源）
        let slices = if level == multi.strategy_level {
            // 情况A：请求的 level 就是策略级别 → 直接用预构建数据
            &sym.strategy
        } else if level == multi.source_level {
            // 情况B：请求的 level 就是源数据级别 → 直接用预构建数据
            &sym.source
        } else {
            // 情况C：其他 level → 从源数据按需重采样
            // 先校验 level 是否可以由 source_level 重采样得到
            if !multi.source_level.is_valid_sampling_target(level) {
                return None;
            }
            let mut cache = sym.level_cache.borrow_mut();
            let cache_idx = cache.iter().position(|(l, _)| *l == level);
            if cache_idx.is_none() {
                // 缓存未命中：重采样并存入缓存
                let resampled = crate::util::resample(sym.source_klines, level).ok()?;
                let mut entry = LevelCacheEntry::default();
                entry.extend_from_klines(&resampled);
                cache.push((level, entry));
            }
            // 生命周期延长：缓存存在 SymbolBuffer → run_multi 的栈上，
            // 比策略调用活得久，这里 transmute 到 'a 是安全的
            let entry: &'a LevelCacheEntry =
                unsafe { std::mem::transmute(&cache.iter().find(|(l, _)| *l == level).unwrap().1) };

            &SymbolSlices {
                time: entry.time.as_slice(),
                open: entry.open.as_slice(),
                high: entry.high.as_slice(),
                low: entry.low.as_slice(),
                close: entry.close.as_slice(),
                volume: entry.volume.as_slice(),
            }
        };

        // 步骤2：过滤并切片辅助 series
        // 只取 (symbol, level) 完全匹配的 series，然后按时间交集切片
        let series: Vec<(&str, &[Decimal])> = multi
            .series
            .iter()
            .filter(|((s, l, _), _)| s == symbol && *l == level)
            .map(|((_, _, name), aligned)| {
                // 时间交集切片：如果 series 覆盖的时间范围
                // 和当前 OHLCV 数据没有重叠，返回空切片
                slice_aligned_series(aligned, name.as_str(), slices.time, level)
            })
            .collect();

        // 步骤3：组装新的 Context 并返回
        Some(Context {
            time: TimeSeries::new(slices.time),
            open: Series::new(slices.open),
            high: Series::new(slices.high),
            low: Series::new(slices.low),
            close: Series::new(slices.close),
            volume: Series::new(slices.volume),
            exchange: multi.exchange,
            series,
            multi: Some(multi),
        })
    }
}
