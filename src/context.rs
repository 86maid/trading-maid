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
    pub(crate) multi: Option<&'a RequestContext<'a>>,
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
        // 只在多标的模式下可用
        let multi = self.multi?;
        // 查找目标标的的数据
        let sym = multi
            .symbol
            .iter()
            .find(|(s, _)| s == symbol)
            .map(|(_, v)| v)?;

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

            let mut cache = sym.level_kline.borrow_mut();
            let cache_idx = cache.iter().position(|(l, _)| *l == level);
            if cache_idx.is_none() {
                // 缓存未命中：重采样并存入缓存
                let resampled = resample(sym.source_kline, level).ok()?;
                let mut entry = LevelKLine::default();
                entry.extend_from_klines(&resampled);
                cache.push((level, entry));
            }
            // 生命周期延长：缓存存在 SymbolBuffer → run_multi 的栈上，
            // 比策略调用活得久，这里 transmute 到 'a 是安全的
            let entry: &'a LevelKLine =
                unsafe { std::mem::transmute(&cache.iter().find(|(l, _)| *l == level).unwrap().1) };

            &RawContext {
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
            .map(|((_, _, name), aligned)| clip_series(aligned, name.as_str(), slices.time, level))
            .collect();

        // 步骤3：组装新的 Context 并返回
        Some(Context {
            time: TimeSeries::new(slices.time),
            open: Series::new(slices.open),
            high: Series::new(slices.high),
            low: Series::new(slices.low),
            close: Series::new(slices.close),
            volume: Series::new(slices.volume),
            exchange: self.exchange,
            series,
            multi: Some(multi),
        })
    }
}

pub(crate) struct RawContext<'a> {
    pub time: &'a [u64],
    pub open: &'a [Decimal],
    pub high: &'a [Decimal],
    pub low: &'a [Decimal],
    pub close: &'a [Decimal],
    pub volume: &'a [Decimal],
}

#[derive(Default)]
pub(crate) struct LevelKLine {
    pub time: Vec<u64>,
    pub open: Vec<Decimal>,
    pub high: Vec<Decimal>,
    pub low: Vec<Decimal>,
    pub close: Vec<Decimal>,
    pub volume: Vec<Decimal>,
}

impl LevelKLine {
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

pub(crate) struct SymbolContext<'a> {
    pub strategy: RawContext<'a>,
    pub source: RawContext<'a>,
    pub source_kline: &'a [KLine],
    pub level_kline: &'a RefCell<Vec<(Level, LevelKLine)>>,
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
