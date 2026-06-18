use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ops::{Deref, Index};

use rust_decimal::Decimal;

use crate::data::{KLine, Level};
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
    pub level_cache: &'a RefCell<BTreeMap<Level, LevelCacheEntry>>,
}

/// Holds data for all symbols in a multi-symbol run.
///
/// Built by [`Engine::run_multi`](crate::engine::Engine::run_multi) before each
/// strategy invocation and exposed via [`Context::multi`].
pub(crate) struct MultiSlices<'a> {
    pub symbols: BTreeMap<String, MultiSymbolData<'a>>,
    pub exchange: &'a ExchangeWrapper,
    pub strategy_level: Level,
    pub source_level: Level,
    pub series: &'a BTreeMap<(String, Level, String), Vec<Decimal>>,
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
        let multi = self.multi?;
        let sym = multi.symbols.get(symbol)?;

        let slices = if level == multi.strategy_level {
            &sym.strategy
        } else if level == multi.source_level {
            &sym.source
        } else {
            // On-demand resample from source klines, cached in level_cache.
            if !multi.source_level.is_valid_sampling_target(level) {
                return None;
            }
            let mut cache = sym.level_cache.borrow_mut();
            if let Entry::Vacant(e) = cache.entry(level) {
                let resampled = crate::util::resample(sym.source_klines, level).ok()?;
                let mut entry = LevelCacheEntry::default();
                entry.extend_from_klines(&resampled);
                e.insert(entry);
            }
            // Extend the lifetime of the cache entry to 'a — safe because the
            // cache lives in SymbolBuffer → run_multi's stack, which outlives
            // the strategy invocation.
            let entry: &'a LevelCacheEntry = unsafe { std::mem::transmute(&cache[&level]) };

            &SymbolSlices {
                time: entry.time.as_slice(),
                open: entry.open.as_slice(),
                high: entry.high.as_slice(),
                low: entry.low.as_slice(),
                close: entry.close.as_slice(),
                volume: entry.volume.as_slice(),
            }
        };

        let len = slices.time.len();
        let series: Vec<(&str, &[Decimal])> = multi
            .series
            .iter()
            .filter(|((s, l, _), _)| s == symbol && *l == level)
            .map(|((_, _, name), data)| (name.as_str(), &data[..len.min(data.len())]))
            .collect();

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
