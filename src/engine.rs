use crate::prelude::*;
use anyhow::anyhow;
use anyhow::bail;
use std::cell::RefCell;
use std::sync::Arc;

/// Default capacity for the k-line buffer in number of k-lines.
pub const DEFAULT_SERIES_MAX_LENGTH: usize = 20000000;

/// Per-symbol buffers used by [`run`](Engine::run) in multi-symbol mode.
///
/// Maintains columnar storage at the source level (built incrementally) and
/// strategy level (built via resample).  A [`Vec<KLine>`] of source klines is
/// kept for on-demand resampling to arbitrary levels via [`Context::request`].
struct SymbolBuffer {
    /// Accumulates source klines for the current strategy bar.
    min_level_accumulator: Vec<KLine>,
    /// All source klines received so far (for on-demand resampling).
    source_klines: Vec<KLine>,
    /// Columnar OHLCV at the source level (incremented each round).
    source_level_buffer: KLineBuffer,
    /// Columnar OHLCV at the strategy level (extended on bar completion).
    strategy_level_buffer: KLineBuffer,
    /// Lazily populated cache for levels other than source / strategy.
    /// `RefCell` provides interior mutability because [`Context::request`] takes `&self`.
    level_cache: RefCell<Vec<(Level, KLineBuffer)>>,
}

impl SymbolBuffer {
    fn new(max_len: usize) -> Self {
        Self {
            min_level_accumulator: Vec::new(),
            source_klines: Vec::new(),
            source_level_buffer: KLineBuffer::new(max_len),
            strategy_level_buffer: KLineBuffer::new(max_len),
            level_cache: RefCell::new(Vec::new()),
        }
    }
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
    /// - `exchange`: the raw [`Exchange`] trait object for querying state.
    ///
    /// Return `Ok(())` to continue, or `Err(...)` to stop the engine.
    async fn next(
        &mut self,
        kline: KLine,
        exchange: Arc<dyn Exchange + 'static>,
    ) -> anyhow::Result<()>;
}

#[async_trait::async_trait(?Send)]
impl<T> HookFn for T
where
    T: AsyncFn(KLine, Arc<dyn Exchange + 'static>) -> anyhow::Result<()>,
{
    async fn next(
        &mut self,
        kline: KLine,
        exchange: Arc<dyn Exchange + 'static>,
    ) -> anyhow::Result<()> {
        self(kline, exchange).await
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
        }
    }

    /// Set a hook that runs after the strategy on **every** source-level k-line.
    ///
    /// The hook receives the raw k-line and an `Arc<dyn Exchange>` for querying
    /// state. Return an error to stop the engine immediately.
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
        if let Some(idx) = self.series.iter().position(|(k, _)| *k == key) {
            self.series[idx].1 = series;
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
    pub async fn run(&mut self, symbol: impl ToStringVec, level: Level) -> anyhow::Result<()> {
        let symbol = symbol.into_vec();

        if symbol.is_empty() {
            bail!("run: symbol is empty");
        }

        if symbol.len() != 1 {
            return self.run_multi(symbol, level).await;
        }

        let symbol = &symbol[0];
        let metadata = self.exchange.get_metadata(symbol).await?;
        let exchange = ExchangeWrapper::new(self.exchange.clone());
        let source_level = if self.exchange.is_live() {
            level
        } else {
            metadata.level
        };

        if source_level.is_valid_sampling_target(level) {
            let mut min_level_buffer = Vec::new();
            let mut max_level_buffer = KLineBuffer::new(N);
            let mut next_time = None;
            let mut prev_time = 0;

            loop {
                match self.exchange.next(symbol, source_level).await? {
                    Some(v) => {
                        if let Some(next_time) = next_time {
                            if v.time != next_time {
                                if v.time == prev_time {
                                    bail!(
                                        "time discontinuity: next() should block until the next k-line is ready, but returned a mismatched time: expected {} ({}), got {} ({})",
                                        next_time,
                                        t2s(next_time),
                                        v.time,
                                        t2s(v.time),
                                    );
                                } else if v.time < prev_time {
                                    bail!(
                                        "time regression: next() should block until the next k-line is ready, but returned a mismatched time: expected {} ({}), got {} ({})",
                                        next_time,
                                        t2s(next_time),
                                        v.time,
                                        t2s(v.time),
                                    );
                                } else {
                                    let gap_start = next_time;
                                    let gap_end = v.time;

                                    let filled_klines = self.exchange.get_kline(symbol, source_level, gap_start, gap_end).await.map_err(|e| {
                                        anyhow!(
                                            "time discontinuity: failed to fill time gap via get_kline(): range [start, end): start {} ({}), end {} ({}): {}",
                                            gap_start,
                                            t2s(gap_start),
                                            gap_end,
                                            t2s(gap_end),
                                            e
                                        )
                                    })?;

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
                                        None => {
                                            bail!(
                                                "time discontinuity: get_kline() returned empty k-line: range [start, end): start {} ({}), end {} ({})",
                                                gap_start,
                                                t2s(gap_start),
                                                gap_end,
                                                t2s(gap_end),
                                            );
                                        }
                                        _ => {}
                                    }

                                    if let Some(last) = filled_klines.last() {
                                        let last_end = get_time_range(last.time, source_level).map_err(|e| {
                                            anyhow!(
                                                "time discontinuity: get_kline() returned k-line with unexpected time: {}: {}",
                                                last.time,
                                                e
                                            )
                                        })?.1;

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

                                    min_level_buffer.extend(filled_klines);
                                }
                            }
                        }

                        next_time = Some(
                            get_time_range(v.time, source_level)
                                .map_err(|e| {
                                    anyhow!(
                                        "next(): returned k-line has unexpected time: {}: {}",
                                        v.time,
                                        e
                                    )
                                })?
                                .1,
                        );

                        prev_time = v.time;
                        min_level_buffer.push(v);

                        if v.time == get_last_time(v.time, source_level, level)? {
                            max_level_buffer.extend(resample(&min_level_buffer, level)?);
                            min_level_buffer.clear();

                            // 自定义系列必须与 OHLCV 数据在时间上有交集才能在策略中访问到，否则返回空切片 []
                            let context = Context {
                                time: TimeSeries::new(&max_level_buffer.time),
                                open: Series::new(&max_level_buffer.open),
                                high: Series::new(&max_level_buffer.high),
                                low: Series::new(&max_level_buffer.low),
                                close: Series::new(&max_level_buffer.close),
                                volume: Series::new(&max_level_buffer.volume),
                                exchange: &exchange,
                                series: SeriesTable(
                                    self.series
                                        .iter()
                                        .filter(|((s, l, _), _)| s == symbol && *l == level)
                                        .map(|((_, _, name), aligned)| {
                                            clip_series(
                                                aligned,
                                                &max_level_buffer.time,
                                                name.as_str(),
                                                level,
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

                        if let Some(hook) = &mut self.hook {
                            hook.next(v, self.exchange.clone()).await?;
                        }
                    }
                    None => return Ok(()),
                }
            }
        } else {
            bail!(
                "invalid sampling target level: min_level: {}, max_level: {}",
                source_level,
                level
            );
        }
    }

    async fn run_multi(&mut self, symbol: impl ToStringVec, level: Level) -> anyhow::Result<()> {
        let symbols = symbol.into_vec();
        let primary = &symbols[0];
        let metadata = self.exchange.get_metadata(primary).await?;
        let exchange = ExchangeWrapper::new(self.exchange.clone());
        let source_level = if self.exchange.is_live() {
            level
        } else {
            metadata.level
        };

        if !source_level.is_valid_sampling_target(level) {
            bail!(
                "run_multi: invalid sampling target level: min_level: {}, max_level: {}",
                source_level,
                level
            );
        }

        let mut buffers = symbols
            .iter()
            .map(|s| (s.clone(), SymbolBuffer::new(N)))
            .collect::<Vec<_>>();

        let mut primary_bar_ready = false;

        loop {
            let mut all_done = true;

            for sym in &symbols {
                if let Some(kline) = self.exchange.next(sym.as_str(), source_level).await? {
                    all_done = false;

                    let buf = buffers
                        .iter_mut()
                        .find(|(s, _)| s == sym)
                        .map(|(_, b)| b)
                        .unwrap();

                    // Keep source klines for on-demand resampling.
                    buf.source_klines.push(kline);
                    // Push to source-level columnar buffer.
                    buf.source_level_buffer.push(kline);
                    // Accumulate for strategy-level resampling.
                    buf.min_level_accumulator.push(kline);

                    if kline.time == get_last_time(kline.time, source_level, level)? {
                        buf.strategy_level_buffer
                            .extend(resample(&buf.min_level_accumulator, level)?);
                        buf.min_level_accumulator.clear();
                        if sym == primary {
                            primary_bar_ready = true;
                        }
                    }

                    if let Some(hook) = &mut self.hook {
                        hook.next(kline, self.exchange.clone()).await?;
                    }
                }
            }

            if all_done {
                return Ok(());
            }

            if primary_bar_ready {
                primary_bar_ready = false;
                self.call_strategy(&buffers, primary, source_level, level, &exchange)
                    .await?;
            }
        }
    }

    async fn call_strategy(
        &mut self,
        buffers: &[(String, SymbolBuffer)],
        primary: &str,
        source_level: Level,
        strategy_level: Level,
        exchange: &ExchangeWrapper,
    ) -> anyhow::Result<()> {
        // 步骤1：为每个标的构建 MultiSymbolData
        let symbol_data: Vec<(String, SymbolContext)> = buffers
            .iter()
            .map(|(sym, buf)| {
                (
                    sym.clone(),
                    SymbolContext {
                        // 策略级别的 OHLCV 切片（预构建，不涉及重采样）
                        strategy_context: KLineContext {
                            time: &buf.strategy_level_buffer.time,
                            open: &buf.strategy_level_buffer.open,
                            high: &buf.strategy_level_buffer.high,
                            low: &buf.strategy_level_buffer.low,
                            close: &buf.strategy_level_buffer.close,
                            volume: &buf.strategy_level_buffer.volume,
                        },
                        // 源级别的 OHLCV 切片（预构建，不涉及重采样）
                        source_context: KLineContext {
                            time: &buf.source_level_buffer.time,
                            open: &buf.source_level_buffer.open,
                            high: &buf.source_level_buffer.high,
                            low: &buf.source_level_buffer.low,
                            close: &buf.source_level_buffer.close,
                            volume: &buf.source_level_buffer.volume,
                        },
                        // 保留源 kline 原始数据，供 Context::request()
                        // 按需重采样到其他 level
                        source_kline: &buf.source_klines,
                        // 按需重采样的缓存，RefCell 提供内部可变性
                        level_kline: &buf.level_cache,
                    },
                )
            })
            .collect();

        // 步骤2：组装 MultiSlices（传递给 Context::request 使用）
        let multi = RequestContext {
            symbol: symbol_data,
            strategy_level,
            source_level,
            series_table: &self.series, // 所有注册的辅助 series
        };

        // 步骤3：取出主标的策略级别切片
        let primary_data = multi
            .symbol
            .iter()
            .find(|(s, _)| s == primary)
            .map(|(_, v)| v)
            .unwrap();
        let primary_slices = &primary_data.strategy_context;
        // 步骤4：构建辅助 series 列表
        //
        // 过滤条件：只取 (symbol == 主标, level == 策略级别) 的 series。
        // 这意味着辅助 series 只在注册时的 level 下可见——
        // 例如资金费率对齐到 Hour1 后，在 Hour4 上下文中不可见。
        // 标量数据无法像 K 线那样重采样，跨 level 暴露没有普适的
        // 正确语义。
        let series_table = SeriesTable(
            self.series
                .iter()
                // 过滤：symbol 和 level 必须同时匹配
                .filter(|((s, l, _), _)| s == primary && *l == strategy_level)
                // 时间交集切片：如果 series 的时间范围和当前 OHLCV 数据
                // 没有重叠，slice_aligned_series 会返回空切片 []
                .map(|((_, _, name), aligned)| {
                    clip_series(aligned, primary_slices.time, name.as_str(), strategy_level)
                })
                .collect(),
        );

        // 步骤5：组装 Context
        //
        // multi: Some(&multi) 使得策略内可以通过
        // `cx.request("ETHUSDT", Hour1)` 访问其他标的的数据
        let context = Context {
            time: TimeSeries::new(primary_slices.time),
            open: Series::new(primary_slices.open),
            high: Series::new(primary_slices.high),
            low: Series::new(primary_slices.low),
            close: Series::new(primary_slices.close),
            volume: Series::new(primary_slices.volume),
            exchange,
            series: series_table,
            request_context: Some(&multi),
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
