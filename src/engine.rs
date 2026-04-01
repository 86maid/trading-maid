use crate::{
    context::Context,
    data::{KLine, KLineBuffer, Level},
    series::{Series, TimeSeries},
    util::{get_last_time, resample},
};
use crate::{exchange::Exchange, strategy::Strategy};
use anyhow::bail;
use std::sync::Arc;

pub const DEFAULT_SERIES_MAX_LENGTH: usize = 10000000;

#[async_trait::async_trait(?Send)]
pub trait HookFn {
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

pub struct Engine<S, const N: usize = DEFAULT_SERIES_MAX_LENGTH> {
    exchange: Arc<dyn Exchange + 'static>,
    strategy: S,
    hook: Option<Box<dyn HookFn>>,
}

impl<S> Engine<S, DEFAULT_SERIES_MAX_LENGTH>
where
    S: Strategy,
{
    pub fn new(exchange: impl Exchange + 'static, strategy: S) -> Self {
        Self::with(exchange, strategy)
    }
}

impl<S, const N: usize> Engine<S, N>
where
    S: Strategy,
{
    pub fn with(exchange: impl Exchange + 'static, strategy: S) -> Self {
        Self {
            exchange: Arc::new(exchange),
            strategy,
            hook: None,
        }
    }

    pub fn hook(&mut self, hook: impl HookFn + 'static) {
        self.hook = Some(Box::new(hook));
    }

    pub async fn run(&mut self, symbol: impl AsRef<str>, level: Level) -> anyhow::Result<()> {
        let symbol = symbol.as_ref();
        let metadata = self.exchange.get_metadata(symbol).await?;

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

                            let context = Context {
                                exchange: &(*self.exchange),
                                time: TimeSeries::new(&max_level_buffer.time),
                                open: Series::new(&max_level_buffer.open),
                                high: Series::new(&max_level_buffer.high),
                                low: Series::new(&max_level_buffer.low),
                                close: Series::new(&max_level_buffer.close),
                                volume: Series::new(&max_level_buffer.volume),
                            };

                            self.strategy.next(&context).await?;
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
