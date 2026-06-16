use crate::{
    context::Context,
    data::{KLine, KLineBuffer, Level},
    prelude::ExchangeWrapper,
    series::{Series, TimeSeries},
    util::{get_last_time, resample},
};
use crate::{exchange::Exchange, strategy::Strategy};
use anyhow::bail;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default capacity for the k-line buffer in number of k-lines.
///
/// 10,000,000 k-lines at 1-minute resolution covers ~19 years of data.
pub const DEFAULT_SERIES_MAX_LENGTH: usize = 10000000;

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
    series: BTreeMap<(String, Level, String), Vec<Decimal>>,
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
            series: BTreeMap::new(),
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
    /// The series data must be pre-aligned to the same `level` as the one passed
    /// to [`run`](Engine::run) — each element corresponds to one bar at that level.
    /// Use [`get_or_download_funding_rate_to_series`](crate::util::get_or_download_funding_rate_to_series)
    /// to download and align funding rate data.
    ///
    /// The series can be accessed inside the strategy via `cx[name]`:
    ///
    /// ```ignore
    /// let fr = cx["funding_rate"][0];  // latest value
    /// ```
    ///
    /// If no series is registered for the current symbol/level/name combination,
    /// `cx[name]` returns an empty series (compare with `== []`).
    pub fn add_series(
        &mut self,
        symbol: impl AsRef<str>,
        level: Level,
        name: impl AsRef<str>,
        series: impl AsRef<[Decimal]>,
    ) {
        self.series.insert(
            (
                symbol.as_ref().to_string(),
                level,
                name.as_ref().to_string(),
            ),
            series.as_ref().to_vec(),
        );
    }

    /// Run the engine: advance through k-lines, resample, and invoke the strategy.
    ///
    /// The exchange provides data at its native level (e.g. 1m). The engine
    /// resamples to `level` (e.g. 1h) and calls the strategy on each completed
    /// bar at that target level.
    ///
    /// Auxiliary series registered via [`add_series`](Engine::add_series) matching
    /// the given symbol and level are synchronised with the OHLCV data and exposed
    /// through the [`Context`].
    ///
    /// # Arguments
    ///
    /// - `symbol`: the trading pair (e.g. `"BTCUSDT"`).
    /// - `level`: the strategy time frame. Must be >= the source data level.
    ///
    /// # Errors
    ///
    /// Returns an error if the target level is finer than the source level,
    /// or if the strategy/hook returns one.
    pub async fn run(&mut self, symbol: impl AsRef<str>, level: Level) -> anyhow::Result<()> {
        let symbol = symbol.as_ref();
        let metadata = self.exchange.get_metadata(symbol).await?;
        let exchange = ExchangeWrapper::new(self.exchange.clone());

        if metadata.level.is_valid_sampling_target(level) {
            let mut min_level_buffer = Vec::new();
            let mut max_level_buffer = KLineBuffer::<N>::new();

            loop {
                match self.exchange.next(symbol, level).await? {
                    Some(v) => {
                        min_level_buffer.push(v);

                        if v.time == get_last_time(v.time, metadata.level, level)? {
                            max_level_buffer.extend(resample(&min_level_buffer, level)?);
                            min_level_buffer.clear();

                            let len = max_level_buffer.time.len();

                            let series: Vec<(&str, &[Decimal])> = self
                                .series
                                .iter()
                                .filter(|((s, l, _), _)| s == symbol && *l == level)
                                .map(|((_, _, name), data)| {
                                    (name.as_str(), &data[..len.min(data.len())])
                                })
                                .collect();

                            let context = Context {
                                time: TimeSeries::new(&max_level_buffer.time),
                                open: Series::new(&max_level_buffer.open),
                                high: Series::new(&max_level_buffer.high),
                                low: Series::new(&max_level_buffer.low),
                                close: Series::new(&max_level_buffer.close),
                                volume: Series::new(&max_level_buffer.volume),
                                exchange: &exchange,
                                series,
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
                metadata.level,
                level
            );
        }
    }
}
