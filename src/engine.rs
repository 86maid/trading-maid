//! The backtest `Engine` that drives a strategy bar by bar and registers custom series.

use crate::prelude::*;
use anyhow::Context as _;
use anyhow::bail;
use std::cell::RefCell;
use std::sync::Arc;

/// Default capacity for the k-line buffer in number of k-lines.
///
/// zero means unlimited.
pub const DEFAULT_SERIES_MAX_LENGTH: usize = 0;

enum Preload {
    KLineCount(usize),
    From(u64),
    Data(DataSource),
}

/// A hook function called after each k-line is processed by the strategy.
///
/// Unlike [`on_error`](Engine::on_error) which only fires on strategy errors,
/// the hook runs on **every** k-line. Use it to stop the engine when a position
/// is liquidated, an order is rejected, or any custom condition is met.
///
/// See [`Engine::hook`] for usage.
#[async_trait::async_trait(?Send)]
pub trait HookFn {
    /// Called after the strategy's `next` on each k-line.
    ///
    /// - `kline`: the k-line that was just processed (at the **source** level, e.g. 1m).
    /// - `missed_kline_list`: source-level k-lines that had to be backfilled from the exchange
    ///   because a network interruption dropped them, in ascending time order. When it is not
    ///   empty the hook's own state must be replayed over it before `kline`; every entry is older
    ///   than `kline`.
    ///
    ///   **An empty list does not mean the previous call was the previous bar.** A hole in the
    ///   data source is skipped rather than backfilled — those bars do not exist, so there is
    ///   nothing to hand over. A hole shows up only as `kline.time` jumping by more than one
    ///   source interval; that is the only way a hook can notice it, and it has to compare
    ///   against the source level from `exchange.get_metadata(symbol)` to get the missing count.
    /// - `exchange`: the exchange handle for querying state.
    ///
    /// Return `Ok(())` to continue, or `Err(...)` to stop the engine.
    async fn next(
        &mut self,
        kline: KLine,
        missed_kline_list: &[KLine],
        exchange: ExchangeWrapper,
    ) -> anyhow::Result<()>;
}

#[async_trait::async_trait(?Send)]
impl<T> HookFn for T
where
    T: AsyncFn(KLine, &[KLine], ExchangeWrapper) -> anyhow::Result<()>,
{
    async fn next(
        &mut self,
        kline: KLine,
        missed_kline_list: &[KLine],
        exchange: ExchangeWrapper,
    ) -> anyhow::Result<()> {
        self(kline, missed_kline_list, exchange).await
    }
}

/// The backtesting / live-trading engine that drives the event loop.
///
/// `Engine` owns the exchange, the strategy, and optional hook/error-handler.
/// Calling [`run`](Engine::run) enters the main loop:
///
/// 1. Read k-lines from the exchange at the **source** level (e.g. 1m).
/// 2. Resample them to the **strategy** level (e.g. 1h), building OHLC series.
/// 3. Call the strategy's [`next`](Strategy::next) on each completed k-line at the strategy level.
/// 4. Call the hook (if set) on each source-level k-line.
///
/// # Type parameters
///
/// - `S`: the strategy type, must implement [`Strategy`].
/// - `N`: k-line buffer capacity in number of bars (default: [`DEFAULT_SERIES_MAX_LENGTH`]).
///
/// # Example
///
/// ```ignore
/// let mut engine = Engine::new(exchange, my_strategy);
/// engine.hook(my_hook);
/// engine.on_error(|e| {
///     eprintln!("strategy error: {e:#}");
///     Ok(()) // continue despite error
/// });
/// engine.run("BTCUSDT", Level::Hour1).await?;
/// ```
pub struct Engine<S, const N: usize = DEFAULT_SERIES_MAX_LENGTH> {
    exchange: Arc<dyn Exchange + 'static>,
    strategy: S,
    hook: Option<Box<dyn HookFn>>,
    on_error: Option<Box<dyn Fn(anyhow::Error) -> anyhow::Result<()>>>,
    series: Vec<((String, Level, String), AlignedSeries)>,
    preload: Option<Preload>,
}

impl<S> Engine<S, DEFAULT_SERIES_MAX_LENGTH>
where
    S: Strategy,
{
    /// Create an engine with the default k-line buffer capacity.
    ///
    /// This is the recommended constructor. Use [`with`](Engine::with) if you
    /// need a custom buffer size.
    pub fn new(exchange: impl Exchange + 'static, strategy: S) -> Self {
        Self::with(exchange, strategy)
    }
}

impl<S, const N: usize> Engine<S, N>
where
    S: Strategy,
{
    /// Create an engine with a custom k-line buffer capacity `N`.
    pub fn with(exchange: impl Exchange + 'static, strategy: S) -> Self {
        Self {
            exchange: Arc::new(exchange),
            strategy,
            hook: None,
            on_error: None,
            series: Vec::new(),
            preload: None,
        }
    }

    /// Set a hook that runs after the strategy on **every** source-level k-line.
    ///
    /// The hook receives the raw k-line, the source-level k-lines that had to be backfilled
    /// because of a network interruption, and an [`ExchangeWrapper`] for querying state.
    /// Return an error to stop the engine immediately.
    ///
    /// Typical use: detect liquidation or rejected orders and abort the backtest.
    pub fn hook(&mut self, hook: impl HookFn + 'static) {
        self.hook = Some(Box::new(hook));
    }

    /// Set an error handler for strategy errors.
    ///
    /// Without this, any error from the strategy stops the engine immediately.
    /// When set, strategy errors are passed to `on_error`; return `Ok(())` to
    /// continue the backtest, or `Err(...)` to stop.
    ///
    /// Unlike [`hook`](Engine::hook), `on_error` is **only** called when the
    /// strategy returns an error.
    pub fn on_error(&mut self, on_error: impl Fn(anyhow::Error) -> anyhow::Result<()> + 'static) {
        self.on_error = Some(Box::new(on_error));
    }

    /// Preload up to `kline_count` closed k-lines at the strategy level before the first live
    /// k-line.
    ///
    /// Preloading gives a freshly started live strategy the history its indicators need, so
    /// `sma(cx.close, 200)` is usable on the very first call instead of returning `None` for
    /// the first 200 k-lines. Without preloading, a restart leaves the strategy blind for as long
    /// as its longest lookback.
    ///
    /// Only applies to live exchanges ([`Exchange::is_live`] returning `true`); it is ignored
    /// in backtests, where the data source starts from its first bar anyway. Replaces any
    /// previous preload configuration.
    pub fn preload(&mut self, kline_count: usize) {
        self.preload = Some(Preload::KLineCount(kline_count));
    }

    /// Preload history from `start_time` (millisecond timestamp) up to the first live k-line.
    ///
    /// `0` means "as early as the data source allows". Replaces any previous preload
    /// configuration.
    pub fn preload_from(&mut self, start_time: u64) {
        self.preload = Some(Preload::From(start_time));
    }

    /// Preload from an in-memory [`DataSource`] instead of fetching through
    /// [`Exchange::get_kline`].
    ///
    /// Useful when the exchange only exposes a shallow history window, or when a known-good
    /// snapshot is preferred over a network round trip at startup. See
    /// [`DataSource::from_file`] to build one from a local file.
    ///
    /// The data must belong to the same symbol passed to [`run`](Engine::run) and be at the
    /// strategy level or finer (finer data is resampled). Every k-line of `data` older than the
    /// first live k-line is preloaded, and any hole between the data's last k-line and the first
    /// live k-line is filled through [`Exchange::get_kline`]. Replaces any previous preload
    /// configuration.
    pub fn preload_data(&mut self, data: DataSource) {
        self.preload = Some(Preload::Data(data));
    }

    /// Register an auxiliary data series to be available in the strategy context.
    ///
    /// The series should be produced by
    /// [`align_to_series`](crate::util::align_to_series) or
    /// [`get_or_download_funding_rate_to_series`](crate::util::get_or_download_funding_rate_to_series),
    /// which embed the target level and time bounds.  At runtime the engine
    /// slices each [`AlignedSeries`] to the time-intersection with the OHLCV
    /// data, so a single download can be re-used across backtests that cover
    /// different (possibly shorter) time windows.
    ///
    /// The [`Level`] is taken from `series.level` — you don't need to pass it
    /// separately.
    ///
    /// The series can be accessed inside the strategy via `cx[name]`:
    ///
    /// ```ignore
    /// let fr = cx["funding_rate"][0];  // latest value
    /// ```
    ///
    /// If no series is registered for the current symbol/level/name
    /// combination, or the time ranges do not overlap, `cx[name]` returns
    /// an empty series (compare with `== []`).
    pub fn add_series(
        &mut self,
        symbol: impl AsRef<str>,
        name: impl AsRef<str>,
        series: AlignedSeries,
    ) {
        let key = (
            symbol.as_ref().to_string(),
            series.level,
            name.as_ref().to_string(),
        );

        if let Some(v) = self.series.iter().position(|(v, _)| *v == key) {
            self.series[v].1 = series;
        } else {
            self.series.push((key, series));
        }
    }

    /// Run the engine: advance through K-lines, resample, and invoke the strategy.
    ///
    /// The exchange provides data at its native level (e.g. 1m). The engine
    /// resamples to `level` (e.g. 1h) and calls the strategy on each completed
    /// bar at that target level.
    ///
    /// # Live vs Backtesting Mode
    ///
    /// The behavior of `exchange.next()` and `exchange.get_kline()` depends on
    /// the exchange's live mode, as determined by [`is_live`](Exchange::is_live):
    ///
    /// - **Live mode (`is_live() == true`)**: The exchange always uses the
    ///   strategy's `level` as the K-line period parameter when calling these methods.
    ///
    /// - **Backtesting mode (`is_live() == false`)**: The exchange uses
    ///   `DataSource.metadata.level` as the K-line period parameter instead.
    ///   If the strategy's `level` exceeds the data source's level, the engine
    ///   automatically resamples the data before passing it to the strategy.
    ///
    /// # Single-symbol mode
    ///
    /// Pass a single symbol (e.g. `"BTCUSDT"`) to run the strategy on one
    /// trading pair.
    ///
    /// # Multi-symbol mode
    ///
    /// Pass multiple symbols (e.g. `["BTCUSDT", "ETHUSDT"]`) to run the engine
    /// across multiple symbols synchronised on a single timeline. Each symbol
    /// gets its own OHLCV buffer; when the primary symbol's bar completes, the
    /// strategy is invoked. Use [`Context::request`] inside the strategy to
    /// read other symbols' data.
    ///
    /// The exchange must be a multi-symbol implementation (e.g.
    /// [`LocalExchange`](crate::local_exchange::LocalExchange)) with
    /// A1 pacemaker semantics: the first symbol in `symbols` drives the clock.
    ///
    /// Auxiliary series registered via [`add_series`](Engine::add_series) matching
    /// the given symbol and level are synchronised with the OHLCV data and exposed
    /// through the [`Context`].
    ///
    /// # Arguments
    ///
    /// - `symbol`: a single trading pair (e.g. `"BTCUSDT"`) or an ordered list
    ///   of trading pairs (e.g. `["BTCUSDT", "ETHUSDT"]`). In multi-symbol mode,
    ///   the first symbol is the primary (its bars gate the strategy call).
    /// - `level`: the strategy time frame. Must be >= the source data level.
    ///
    /// # Errors
    ///
    /// Returns an error if the target level is finer than the source level,
    /// or if the strategy/hook returns one.
    pub async fn run(
        &mut self,
        symbol: impl ToStringVec,
        strategy_level: Level,
    ) -> anyhow::Result<()> {
        let symbol = symbol.into_vec();

        if symbol.is_empty() {
            bail!("run: symbol is empty");
        }

        if symbol.len() != 1 {
            return self.run_multi(symbol, strategy_level).await;
        }

        let symbol = &symbol[0];
        let metadata = self.exchange.get_metadata(symbol).await?;
        let exchange = ExchangeWrapper::new(self.exchange.clone());
        let source_level = if self.exchange.is_live() {
            strategy_level
        } else {
            metadata.level
        };

        if source_level.is_valid_sampling_target(strategy_level) {
            let mut source_level_kline_buffer = Vec::new();
            let mut strategy_level_kline_buffer = KLineBuffer::new(N);
            let mut next_time = None;
            let mut prev_time = 0;
            let mut preloaded = false;
            let mut pending_missing = 0usize;
            let mut contaminated: Option<(u64, u64)> = None;

            loop {
                match self.exchange.next(symbol, source_level).await? {
                    Some(kline) => {
                        if !preloaded {
                            preloaded = true;

                            strategy_level_kline_buffer.extend(
                                self.build_preload(symbol, source_level, kline.time).await?,
                            );
                        }

                        let (filled, new_next_time, gap) = check_kline(
                            self.exchange.clone(),
                            symbol,
                            source_level,
                            kline,
                            next_time,
                            prev_time,
                        )
                        .await?;

                        next_time = Some(new_next_time);
                        prev_time = kline.time;

                        let missed_kline_list = filled.clone().unwrap_or_default();

                        if let Some(filled) = filled {
                            source_level_kline_buffer.extend(filled);
                        }

                        // Hole in the source data: remember which strategy-level buckets it
                        // touches, and how many strategy-level bars they amount to.
                        if let Some((gap_start, gap_end)) = gap {
                            let (first_start, _) = get_time_range(gap_start, strategy_level)?;
                            let (last_start, last_end) =
                                get_time_range(gap_end - 1, strategy_level)?;

                            // Walk bucket by bucket instead of dividing by the interval: a
                            // calendar month has no fixed millisecond length.
                            let mut count = 1usize;
                            let mut cursor = first_start;

                            while cursor < last_start {
                                cursor = get_time_range(cursor, strategy_level)?.1;

                                count += 1;
                            }

                            pending_missing += count;

                            contaminated = Some(match contaminated {
                                Some((start, end)) => (start.min(first_start), end.max(last_end)),
                                None => (first_start, last_end),
                            });
                        }

                        source_level_kline_buffer.push(kline);

                        if kline.time == get_last_time(kline.time, source_level, strategy_level)? {
                            // Drop the source bars of every contaminated bucket before resampling:
                            // a bucket missing any of its source bars has a wrong high/low/volume,
                            // and it would still look perfectly contiguous to the strategy.
                            if let Some((start, end)) = contaminated {
                                source_level_kline_buffer
                                    .retain(|v| v.time < start || v.time >= end);
                            }

                            let new_bar_list =
                                resample(&source_level_kline_buffer, strategy_level)?;

                            source_level_kline_buffer.clear();

                            // The bucket that just closed is past the contaminated span.
                            if let Some((_, end)) = contaminated
                                && get_time_range(kline.time, strategy_level)?.1 >= end
                            {
                                contaminated = None;
                            }

                            // Every bucket of this close was contaminated: no new bar, so the
                            // strategy is not called and `pending_missing` waits for the next one.
                            if !new_bar_list.is_empty() {
                                let missed = if pending_missing > 0 {
                                    Some(Missed::Missing(std::mem::take(&mut pending_missing)))
                                } else if new_bar_list.len() > 1 {
                                    Some(Missed::Backfilled(new_bar_list.len() - 1))
                                } else {
                                    None
                                };

                                strategy_level_kline_buffer.extend(new_bar_list);

                                // 自定义系列必须与 OHLCV 数据在时间上有交集才能在策略中访问到，否则返回空切片 []
                                let window = strategy_level_kline_buffer.window();

                                let context = Context {
                                    time: TimeSeries::new(window.time),
                                    open: Series::new(window.open),
                                    high: Series::new(window.high),
                                    low: Series::new(window.low),
                                    close: Series::new(window.close),
                                    volume: Series::new(window.volume),
                                    missed,
                                    exchange: &exchange,
                                    series: SeriesTable(
                                        self.series
                                            .iter()
                                            .filter(|((s, l, _), _)| {
                                                s == symbol && *l == strategy_level
                                            })
                                            .map(|((_, _, name), aligned)| {
                                                clip_series(
                                                    aligned,
                                                    window.time,
                                                    name.as_str(),
                                                    strategy_level,
                                                )
                                            })
                                            .collect(),
                                    ),
                                    request_context: None,
                                };

                                if let Err(v) = self.strategy.next(&context).await {
                                    if let Some(on_error) = &self.on_error {
                                        on_error(v)?;
                                    } else {
                                        bail!(v);
                                    }
                                }
                            }
                        }

                        if let Some(hook) = &mut self.hook {
                            hook.next(
                                kline,
                                &missed_kline_list,
                                ExchangeWrapper::new(self.exchange.clone()),
                            )
                            .await?;
                        }
                    }
                    None => return Ok(()),
                }
            }
        } else {
            bail!(
                "invalid sampling target level: source level: {}, strategy level: {}",
                source_level,
                strategy_level
            );
        }
    }

    async fn run_multi(
        &mut self,
        symbol: impl ToStringVec,
        strategy_level: Level,
    ) -> anyhow::Result<()> {
        let symbol_list = symbol.into_vec();
        let primary = &symbol_list[0];
        let metadata = self.exchange.get_metadata(primary).await?;
        let exchange = ExchangeWrapper::new(self.exchange.clone());
        let source_level = if self.exchange.is_live() {
            strategy_level
        } else {
            metadata.level
        };

        if !source_level.is_valid_sampling_target(strategy_level) {
            bail!(
                "run_multi: invalid sampling target level: source level: {}, strategy level: {}",
                source_level,
                strategy_level
            );
        }

        struct RunState {
            next_time: Option<u64>,
            prev_time: u64,
            eof: bool,
            preloaded: bool,
        }

        let mut symbol_buffer: Vec<(String, SymbolBuffer)> = symbol_list
            .iter()
            .map(|s| (s.clone(), SymbolBuffer::new(N)))
            .collect();

        let mut states = symbol_list
            .iter()
            .map(|_| RunState {
                next_time: None,
                prev_time: 0,
                eof: false,
                preloaded: false,
            })
            .collect::<Vec<_>>();

        let mut ready = false;

        async fn step(
            exchange: Arc<dyn Exchange + 'static>,
            symbol: String,
            source_level: Level,
            next_time: Option<u64>,
            prev_time: u64,
        ) -> anyhow::Result<Option<(KLine, Option<Vec<KLine>>, u64, Option<(u64, u64)>)>> {
            let Some(kline) = exchange.next(&symbol, source_level).await? else {
                return Ok(None);
            };

            let (filled, new_next_time, gap) =
                check_kline(exchange, &symbol, source_level, kline, next_time, prev_time).await?;

            Ok(Some((kline, filled, new_next_time, gap)))
        }

        loop {
            let mut pending = Vec::with_capacity(symbol_list.len());

            for (i, symbol) in symbol_list.iter().enumerate() {
                if states[i].eof {
                    continue;
                }

                pending.push((i, symbol.clone(), states[i].next_time, states[i].prev_time));
            }

            if pending.is_empty() {
                return Ok(());
            }

            let mut task_output_list = Vec::with_capacity(pending.len());

            if self.exchange.is_live() {
                let mut handle_list = Vec::with_capacity(pending.len());

                for (i, symbol, next_time, prev_time) in pending {
                    let ex = self.exchange.clone();

                    handle_list.push(tokio::spawn(async move {
                        let result = step(ex, symbol, source_level, next_time, prev_time).await;

                        (i, result)
                    }));
                }

                for handle in handle_list {
                    let (i, result) = handle.await?;

                    task_output_list.push((i, result));
                }
            } else {
                for (i, symbol, next_time, prev_time) in pending {
                    let result = step(
                        self.exchange.clone(),
                        symbol,
                        source_level,
                        next_time,
                        prev_time,
                    )
                    .await;

                    task_output_list.push((i, result));
                }
            }

            let mut all_eof = true;

            for (symbol_index, result) in task_output_list {
                match result? {
                    Some((current, filled, new_next_time, gap)) => {
                        all_eof = false;

                        if !states[symbol_index].preloaded {
                            states[symbol_index].preloaded = true;

                            let preload = self
                                .build_preload(
                                    &symbol_list[symbol_index],
                                    source_level,
                                    current.time,
                                )
                                .await?;

                            symbol_buffer[symbol_index]
                                .1
                                .strategy_level_kline_buffer
                                .extend(preload);
                        }

                        let state = &mut states[symbol_index];
                        let buffer = &mut symbol_buffer[symbol_index].1;

                        state.next_time = Some(new_next_time);
                        state.prev_time = current.time;
                        buffer.missed = None;

                        let missed_kline_list = filled.clone().unwrap_or_default();

                        if let Some(filled) = filled {
                            for k in &filled {
                                buffer.source_kline_resample_buffer.push(*k);
                                buffer.source_kline_buffer.push(*k);
                            }
                            buffer.source_level_kline_buffer.extend(filled);
                        }

                        // Hole in the source data: remember which strategy-level buckets it
                        // touches, and how many strategy-level bars they amount to.
                        if let Some((gap_start, gap_end)) = gap {
                            let (first_start, _) = get_time_range(gap_start, strategy_level)?;
                            let (last_start, last_end) =
                                get_time_range(gap_end - 1, strategy_level)?;

                            // Walk bucket by bucket instead of dividing by the interval: a
                            // calendar month has no fixed millisecond length.
                            let mut count = 1usize;
                            let mut cursor = first_start;

                            while cursor < last_start {
                                cursor = get_time_range(cursor, strategy_level)?.1;

                                count += 1;
                            }

                            buffer.pending_missing += count;

                            buffer.contaminated = Some(match buffer.contaminated {
                                Some((start, end)) => (start.min(first_start), end.max(last_end)),
                                None => (first_start, last_end),
                            });
                        }

                        buffer.source_kline_resample_buffer.push(current);
                        buffer.source_kline_buffer.push(current);
                        buffer.source_level_kline_buffer.push(current);

                        if current.time
                            == get_last_time(current.time, source_level, strategy_level)?
                        {
                            // Drop the source bars of every contaminated bucket before resampling:
                            // a bucket missing any of its source bars has a wrong high/low/volume,
                            // and it would still look perfectly contiguous to the strategy.
                            if let Some((start, end)) = buffer.contaminated {
                                buffer
                                    .source_kline_resample_buffer
                                    .retain(|v| v.time < start || v.time >= end);
                            }

                            let new_bar_list =
                                resample(&buffer.source_kline_resample_buffer, strategy_level)?;

                            buffer.source_kline_resample_buffer.clear();

                            // The bucket that just closed is past the contaminated span.
                            if let Some((_, end)) = buffer.contaminated
                                && get_time_range(current.time, strategy_level)?.1 >= end
                            {
                                buffer.contaminated = None;
                            }

                            // Every bucket of this close was contaminated: no new bar, so the
                            // strategy is not called and `pending_missing` waits for the next one.
                            if !new_bar_list.is_empty() {
                                buffer.missed = if buffer.pending_missing > 0 {
                                    Some(Missed::Missing(std::mem::take(
                                        &mut buffer.pending_missing,
                                    )))
                                } else if new_bar_list.len() > 1 {
                                    Some(Missed::Backfilled(new_bar_list.len() - 1))
                                } else {
                                    None
                                };

                                buffer.strategy_level_kline_buffer.extend(new_bar_list);

                                if symbol_list[symbol_index] == *primary {
                                    ready = true;
                                }
                            }
                        }

                        if let Some(hook) = &mut self.hook {
                            hook.next(
                                current,
                                &missed_kline_list,
                                ExchangeWrapper::new(self.exchange.clone()),
                            )
                            .await?;
                        }
                    }
                    None => {
                        states[symbol_index].eof = true;
                        symbol_buffer[symbol_index].1.eof = true;

                        if symbol_list[symbol_index] == *primary {
                            return Ok(());
                        }
                    }
                }
            }

            if all_eof {
                return Ok(());
            }

            if ready {
                ready = false;

                self.call_strategy(
                    &symbol_buffer,
                    primary,
                    source_level,
                    strategy_level,
                    &exchange,
                )
                .await?;
            }
        }
    }

    async fn call_strategy(
        &mut self,
        symbol_buffer: &[(String, SymbolBuffer)],
        primary: &str,
        source_level: Level,
        strategy_level: Level,
        exchange: &ExchangeWrapper,
    ) -> anyhow::Result<()> {
        let request_context = RequestContext {
            symbol: symbol_buffer
                .iter()
                .filter(|(_, v)| !v.eof)
                .map(|(s, v)| (s.clone(), v.as_context()))
                .collect(),
            strategy_level,
            source_level,
            series: &self.series,
            max_size: N,
        };

        let primary_symbol_context = request_context
            .symbol
            .iter()
            .find(|(s, _)| s == primary)
            .map(|(_, v)| v)
            .unwrap();

        let primary_kline_context = &primary_symbol_context.strategy_level_kline_context;

        // 自定义系列必须与 OHLCV 数据在时间上有交集才能在策略中访问到，否则返回空切片 []
        let context = Context {
            time: TimeSeries::new(primary_kline_context.time),
            open: Series::new(primary_kline_context.open),
            high: Series::new(primary_kline_context.high),
            low: Series::new(primary_kline_context.low),
            close: Series::new(primary_kline_context.close),
            volume: Series::new(primary_kline_context.volume),
            missed: primary_symbol_context.missed,
            exchange,
            series: SeriesTable(
                self.series
                    .iter()
                    .filter(|((s, l, _), _)| s == primary && *l == strategy_level)
                    .map(|((_, _, name), aligned)| {
                        clip_series(
                            aligned,
                            primary_kline_context.time,
                            name.as_str(),
                            strategy_level,
                        )
                    })
                    .collect(),
            ),
            request_context: Some(&request_context),
        };

        if let Err(e) = self.strategy.next(&context).await {
            if let Some(on_error) = &self.on_error {
                on_error(e)?;
            } else {
                bail!(e);
            }
        }

        Ok(())
    }

    async fn build_preload(
        &self,
        symbol: &str,
        level: Level,
        first_time: u64,
    ) -> anyhow::Result<Vec<KLine>> {
        if !self.exchange.is_live() || self.preload.is_none() {
            return Ok(Vec::new());
        }

        fn shift_back(time: u64, level: Level, count: usize) -> anyhow::Result<u64> {
            if level != Level::Month1 {
                let span = (count as u64).saturating_mul(level.interval_millis());

                return Ok(get_time_range(time.saturating_sub(span), level)?.0);
            }

            let mut start = time;

            for _ in 0..count {
                start = get_time_range(start.saturating_sub(1), level)?.0;
            }

            Ok(start)
        }

        let Some(preload) = &self.preload else {
            return Ok(Vec::new());
        };

        let (start, seed) = match preload {
            Preload::KLineCount(kline_count) => {
                (shift_back(first_time, level, *kline_count)?, None)
            }
            Preload::From(start_time) => {
                let start = get_time_range(*start_time, level)?.0;

                if start >= first_time {
                    return Ok(Vec::new());
                }

                (start, None)
            }
            Preload::Data(data) => (0, Some(data)),
        };

        let mut kline_list = match seed {
            Some(data) => {
                if data.metadata.symbol != symbol {
                    bail!(
                        "preload_data: symbol mismatch: expected {}, got {}",
                        symbol,
                        data.metadata.symbol
                    );
                }

                let data = if data.metadata.level == level {
                    data.clone()
                } else {
                    data.resample(level).context(format!(
                        "preload_data: cannot resample {} data to {}",
                        data.metadata.level, level
                    ))?
                };

                data.range(start, first_time).data.clone()
            }
            None => Vec::new(),
        };

        let fill_start = match kline_list.last() {
            Some(last) => get_time_range(last.time, level)?.1.max(start),
            None => start,
        };

        if fill_start < first_time {
            kline_list.extend(
                self.exchange
                    .get_kline(symbol, level, fill_start, first_time)
                    .await?,
            );
        }

        if let Some(last) = kline_list.last() {
            let next = get_time_range(last.time, level)?.1;

            if next != first_time {
                bail!(
                    "preload: last k-line {} ({}) is not adjacent to the first live k-line {} ({})",
                    last.time,
                    t2s(last.time),
                    first_time,
                    t2s(first_time),
                );
            }
        }

        Ok(kline_list)
    }
}

pub trait ToStringVec {
    fn into_vec(self) -> Vec<String>;
}

impl ToStringVec for &str {
    fn into_vec(self) -> Vec<String> {
        vec![self.to_string()]
    }
}

impl ToStringVec for String {
    fn into_vec(self) -> Vec<String> {
        vec![self]
    }
}

impl<U> ToStringVec for U
where
    U: IsContainer + IntoIterator,
    U::Item: ToString,
{
    fn into_vec(self) -> Vec<String> {
        self.into_iter().map(|item| item.to_string()).collect()
    }
}

struct SymbolBuffer {
    source_kline_resample_buffer: Vec<KLine>,
    source_kline_buffer: Vec<KLine>,
    source_level_kline_buffer: KLineBuffer,
    strategy_level_kline_buffer: KLineBuffer,
    level_kline_table: RefCell<Vec<LevelKLine>>,
    missed: Option<Missed>,
    /// Missing steps already detected but not delivered yet: the hole swallowed every bucket of
    /// the last close, so no new bar triggered a strategy call. Delivered on the next one.
    pending_missing: usize,
    /// Strategy-level bucket span `[start, end)` touched by a data hole. Source bars inside it
    /// are dropped before resampling. Cleared once a close is past the span.
    contaminated: Option<(u64, u64)>,
    eof: bool,
}

impl SymbolBuffer {
    fn new(max_len: usize) -> Self {
        Self {
            source_kline_resample_buffer: Vec::new(),
            source_kline_buffer: Vec::new(),
            source_level_kline_buffer: KLineBuffer::new(max_len),
            strategy_level_kline_buffer: KLineBuffer::new(max_len),
            level_kline_table: RefCell::new(Vec::new()),
            missed: None,
            pending_missing: 0,
            contaminated: None,
            eof: false,
        }
    }

    fn as_context(&self) -> SymbolContext<'_> {
        let strategy_window = self.strategy_level_kline_buffer.window();
        let source_window = self.source_level_kline_buffer.window();

        SymbolContext {
            strategy_level_kline_context: KLineContext {
                time: strategy_window.time,
                open: strategy_window.open,
                high: strategy_window.high,
                low: strategy_window.low,
                close: strategy_window.close,
                volume: strategy_window.volume,
            },
            source_level_kline_context: KLineContext {
                time: source_window.time,
                open: source_window.open,
                high: source_window.high,
                low: source_window.low,
                close: source_window.close,
                volume: source_window.volume,
            },
            source_kline_buffer: &self.source_kline_buffer,
            level_kline_table: &self.level_kline_table,
            missed: self.missed,
        }
    }
}

/// Validate that the k-line returned by `next()` is contiguous with the previous one, and
/// handle the fill when it is not.
///
/// The third return value is the hole that could not be filled: only a local exchange produces
/// it — its data is a read-only snapshot, so nothing can be filled in and the caller must skip
/// the hole. A live exchange that fails to fill a hole is still an error.
async fn check_kline(
    exchange: Arc<dyn Exchange + 'static>,
    symbol: &str,
    source_level: Level,
    kline: KLine,
    next_time: Option<u64>,
    prev_time: u64,
) -> anyhow::Result<(Option<Vec<KLine>>, u64, Option<(u64, u64)>)> {
    let mut gap = None;

    let filled = if let Some(nt) = next_time {
        if kline.time != nt {
            if kline.time == prev_time {
                bail!(
                    "time discontinuity: next() should block until the next k-line is ready, but returned a mismatched time: expected {} ({}), got {} ({})",
                    nt,
                    t2s(nt),
                    kline.time,
                    t2s(kline.time),
                );
            } else if kline.time < prev_time {
                bail!(
                    "time regression: next() should block until the next k-line is ready, but returned a mismatched time: expected {} ({}), got {} ({})",
                    nt,
                    t2s(nt),
                    kline.time,
                    t2s(kline.time),
                );
            }

            let gap_start = nt;
            let gap_end = kline.time;

            let filled_klines = exchange
                .get_kline(symbol, source_level, gap_start, gap_end)
                .await
                .context(format!(
                    "time discontinuity: failed to fill time gap via get_kline(): range [start, end): start {} ({}), end {} ({})",
                    gap_start,
                    t2s(gap_start),
                    gap_end,
                    t2s(gap_end),
                ))?;

            match filled_klines.first() {
                Some(first) if first.time != gap_start => {
                    bail!(
                        "time discontinuity: get_kline() returned unexpected first k-line: expected {} ({}), got {} ({})",
                        gap_start,
                        t2s(gap_start),
                        first.time,
                        t2s(first.time),
                    );
                }
                // live: the bars should exist but could not be backfilled — that is a failure.
                None if exchange.is_live() => {
                    bail!(
                        "time discontinuity: get_kline() returned empty k-line: range [start, end): start {} ({}), end {} ({})",
                        gap_start,
                        t2s(gap_start),
                        gap_end,
                        t2s(gap_end),
                    );
                }
                // local: the bars genuinely do not exist. Report the hole and let the caller skip it.
                None => {
                    gap = Some((gap_start, gap_end));
                }
                _ => {}
            }

            if gap.is_none()
                && let Some(last) = filled_klines.last()
            {
                let last_end = get_time_range(last.time, source_level)
                    .context(format!(
                        "time discontinuity: get_kline() returned k-line with unexpected time: {}",
                        last.time,
                    ))?
                    .1;

                if last_end != gap_end {
                    bail!(
                        "time discontinuity: get_kline() returned unexpected last k-line: expected end {} ({}), got end {} ({})",
                        gap_end,
                        t2s(gap_end),
                        last_end,
                        t2s(last_end),
                    );
                }
            }

            Some(filled_klines)
        } else {
            None
        }
    } else {
        None
    };

    let new_next_time = get_time_range(kline.time, source_level)
        .context(format!(
            "next(): returned k-line has unexpected time: {}",
            kline.time,
        ))?
        .1;

    Ok((filled, new_next_time, gap))
}
