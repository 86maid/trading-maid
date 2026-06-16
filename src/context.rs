use crate::exchange::*;
use crate::series::*;
use rust_decimal::Decimal;
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
}

impl<'a> Deref for Context<'a> {
    type Target = ExchangeWrapper;

    fn deref(&self) -> &Self::Target {
        &self.exchange
    }
}

/// Access a named series. Returns an empty [`Series`] if the key is not found.
///
/// ```ignore
/// // Check existence
/// if cx["funding_rate"] == [] {
///     return Ok(());
/// }
/// // Read the latest value
/// let fr = cx["funding_rate"][0];
/// ```
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
