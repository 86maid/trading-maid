use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::series::Series;

/// Calculates the Simple Moving Average (SMA) for a given series of prices and a specified length.
/// Returns Some(SMA) or None if the length is zero or if there are not enough data points in the series.
pub fn ma(series: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || series.len() < length {
        return None;
    }

    Some(series.iter().take(length).sum::<Decimal>() / Decimal::from(length))
}

/// Calculates the Exponential Moving Average (EMA) for a given series of prices and a specified length.
/// Returns Some(EMA) or None if the length is zero or if there are not enough data points in the series.
pub fn ema(series: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || series.len() < length {
        return None;
    }

    let mut ema_cache = EMACache::new(length);

    for &price in series.iter().rev() {
        ema_cache.update(price);
    }

    ema_cache.value()
}

/// Calculates the Relative Strength Index (RSI) for a given series of prices and a specified length.
/// Returns Some(RSI) or None if the length is zero or if there are not enough data points in the series.
pub fn rsi(series: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || series.len() < length + 1 {
        return None;
    }

    let mut rsi_cache = RSICache::new(length);

    for &price in series.iter().rev() {
        rsi_cache.update(price);
    }

    rsi_cache.value()
}

/// Calculates the Commodity Channel Index (CCI) for given high, low, and close price series and a specified length.
/// Returns Some(CCI) or None if the length is zero or if there are not enough data points in any of the series.
pub fn cci(high: &Series, low: &Series, close: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || high.len() < length || low.len() < length || close.len() < length {
        return None;
    }

    let typical_prices: Vec<Decimal> = high
        .iter()
        .zip(low.iter())
        .zip(close.iter())
        .take(length)
        .map(|((h, l), c)| (h + l + c) / dec!(3))
        .collect();

    let sma_tp: Decimal = typical_prices.iter().sum::<Decimal>() / Decimal::from(length);
    let mean_deviation = typical_prices
        .iter()
        .map(|tp| (tp - sma_tp).abs())
        .sum::<Decimal>()
        / Decimal::from(length);

    if mean_deviation.is_zero() {
        Some(dec!(0))
    } else {
        Some((typical_prices[0] - sma_tp) / (dec!(0.015) * mean_deviation))
    }
}

/// Calculates the MACD, Signal Line, and Histogram values for a given series of prices.
/// Returns a tuple containing the MACD value, Signal Line value, and Histogram value.
pub fn macd(
    series: &Series,
    fast_length: usize,
    slow_length: usize,
    signal_length: usize,
) -> (Option<Decimal>, Option<Decimal>, Option<Decimal>) {
    if fast_length == 0 || slow_length == 0 || signal_length == 0 || fast_length >= slow_length {
        return (None, None, None);
    }

    let mut fast_ema = EMACache::new(fast_length);
    let mut slow_ema = EMACache::new(slow_length);
    let mut signal_ema = EMACache::new(signal_length);

    for &price in series.iter().rev() {
        let fast = fast_ema.update(price);
        let slow = slow_ema.update(price);

        if fast.is_some() && slow.is_some() {
            let macd_val = fast.unwrap() - slow.unwrap();
            signal_ema.update(macd_val);
        }
    }

    let fast_val = fast_ema.value();
    let slow_val = slow_ema.value();
    let signal_val = signal_ema.value();

    match (fast_val, slow_val, signal_val) {
        (Some(fast), Some(slow), Some(signal)) => {
            let macd_val = fast - slow;
            let histogram = Some(macd_val - signal);
            (Some(macd_val), Some(signal), histogram)
        }
        _ => (None, None, None),
    }
}

/// Calculates the highest value in a given series of prices for a specified length.
/// Returns Some(highest) or None if the length is zero or if there are not enough data points in the series.
pub fn highest(series: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || series.len() < length {
        return None;
    }

    series
        .iter()
        .take(length)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .copied()
}

/// Calculates the lowest value in a given series of prices for a specified length.
/// Returns Some(lowest) or None if the length is zero or if there are not enough data points in the series.
pub fn lowest(series: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || series.len() < length {
        return None;
    }

    series
        .iter()
        .take(length)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .copied()
}

pub struct EMACache {
    length: usize,
    multiplier: Decimal,
    current_ema: Option<Decimal>,
    count: usize,
    sum: Decimal,
}

impl EMACache {
    pub fn new(length: usize) -> Self {
        let multiplier = dec!(2) / (Decimal::from(length) + dec!(1));

        EMACache {
            length,
            multiplier,
            current_ema: None,
            count: 0,
            sum: dec!(0),
        }
    }

    pub fn with_ema(length: usize, ema: impl Into<Decimal>) -> Self {
        let multiplier = dec!(2) / (Decimal::from(length) + dec!(1));

        EMACache {
            length,
            multiplier,
            current_ema: Some(ema.into()),
            count: usize::MAX,
            sum: dec!(0),
        }
    }

    pub fn update(&mut self, price: impl Into<Decimal>) -> Option<Decimal> {
        let price = price.into();

        self.count = self.count.saturating_add(1);

        if self.count < self.length {
            self.sum += price;
            None
        } else if self.count == self.length {
            self.sum += price;
            self.current_ema = Some(self.sum / Decimal::from(self.length));
            self.current_ema
        } else {
            if let Some(current) = self.current_ema {
                self.current_ema =
                    Some(price * self.multiplier + current * (dec!(1) - self.multiplier));
            } else {
                // Fallback to SMA if EMA is not set
                self.current_ema = Some(self.sum / Decimal::from(self.length));
            }
            self.current_ema
        }
    }

    pub fn value(&self) -> Option<Decimal> {
        if self.count != usize::MAX && self.count < self.length {
            None
        } else {
            self.current_ema
        }
    }

    pub fn reset(&mut self) {
        self.count = 0;
        self.sum = dec!(0);
        self.current_ema = None;
    }
}

pub struct RSICache {
    length: usize,
    count: usize,
    last_price: Option<Decimal>,
    sum_gain: Decimal,
    sum_loss: Decimal,
    avg_gain: Decimal,
    avg_loss: Decimal,
}

impl RSICache {
    pub fn new(length: usize) -> Self {
        RSICache {
            length,
            count: 0,
            last_price: None,
            sum_gain: dec!(0),
            sum_loss: dec!(0),
            avg_gain: dec!(0),
            avg_loss: dec!(0),
        }
    }

    pub fn update(&mut self, price: impl Into<Decimal>) -> Option<Decimal> {
        let price = price.into();

        if self.length == 0 {
            return None;
        }

        match self.last_price {
            None => {
                self.last_price = Some(price);
                None
            }
            Some(last) => {
                let delta = price - last;
                self.last_price = Some(price);

                let gain = if delta > dec!(0) { delta } else { dec!(0) };
                let loss = if delta < dec!(0) { -delta } else { dec!(0) };

                self.count = self.count.saturating_add(1);

                if self.count < self.length {
                    self.sum_gain += gain;
                    self.sum_loss += loss;
                    None
                } else if self.count == self.length {
                    self.sum_gain += gain;
                    self.sum_loss += loss;
                    self.avg_gain = self.sum_gain / Decimal::from(self.length);
                    self.avg_loss = self.sum_loss / Decimal::from(self.length);
                    self.value()
                } else {
                    self.avg_gain = (self.avg_gain * (Decimal::from(self.length) - dec!(1)) + gain)
                        / Decimal::from(self.length);
                    self.avg_loss = (self.avg_loss * (Decimal::from(self.length) - dec!(1)) + loss)
                        / Decimal::from(self.length);
                    self.value()
                }
            }
        }
    }

    pub fn value(&self) -> Option<Decimal> {
        if self.count < self.length {
            return None;
        }

        if self.avg_loss.is_zero() {
            return if self.avg_gain.is_zero() {
                Some(dec!(50))
            } else {
                Some(dec!(100))
            };
        }

        let rs = self.avg_gain / self.avg_loss;
        Some(dec!(100) - dec!(100) / (dec!(1) + rs))
    }
}

/// Detect a **swing high** (local maximum) in a price series.
///
/// A swing high is identified when the middle element is strictly greater
/// than all elements on its left and right within the given windows.
///
/// # Arguments
///
/// * `series` – Slice of price data (e.g. highs).
/// * `left`  – Number of bars to inspect before the candidate bar (left side) and the candidate position.
/// * `right` – Number of bars to inspect after the candidate bar (right side).
///
/// # Returns
///
/// Returns `Some((mid, left_min, right_min))` if a swing high is found:
///
/// * `mid` – The swing high value.
/// * `left_min` – Minimum value in the left window (older bars).
/// * `right_min` – Minimum value in the right window (newer bars).
///
/// Returns `None` if:
/// * `left == 0` or `right == 0`
/// * The series is too short
/// * The middle value is not strictly greater than both neighbors
pub fn swing_high(
    series: &Series,
    left: usize,
    right: usize,
) -> Option<(Decimal, Decimal, Decimal)> {
    if left == 0 || right == 0 || series.len() < left + right + 1 {
        return None;
    }

    let mid = &series[left];
    let left_array = &series[left + 1..left + 1 + right];
    let right_array = &series[..left];

    if left_array.iter().all(|x| x < mid) && right_array.iter().all(|x| x < mid) {
        Some((
            *mid,
            left_array.iter().min().copied()?,
            right_array.iter().min().copied()?,
        ))
    } else {
        None
    }
}

/// Detect a **swing low** (local minimum) in a price series.
///
/// A swing low is identified when the middle element is strictly less
/// than all elements on its left and right within the given windows.
///
/// # Arguments
///
/// * `series` – Slice of price data (e.g. lows).
/// * `left`  – Number of bars to inspect before the candidate bar (left side) and the candidate position.
/// * `right` – Number of bars to inspect after the candidate bar (right side).
///
/// # Returns
///
/// Returns `Some((mid, left_max, right_max))` if a swing low is found:
///
/// * `mid` – The swing low value.
/// * `left_max` – Maximum value in the left window (older bars).
/// * `right_max` – Maximum value in the right window (newer bars).
///
/// Returns `None` if:
/// * `left == 0` or `right == 0`
/// * The series is too short
/// * The middle value is not strictly smaller than both neighbors
pub fn swing_low(
    series: &Series,
    left: usize,
    right: usize,
) -> Option<(Decimal, Decimal, Decimal)> {
    if left == 0 || right == 0 || series.len() < left + right + 1 {
        return None;
    }

    let mid = &series[left];
    let left_array = &series[left + 1..left + 1 + right];
    let right_array = &series[..left];

    if left_array.iter().all(|x| x > mid) && right_array.iter().all(|x| x > mid) {
        Some((
            *mid,
            left_array.iter().max().copied()?,
            right_array.iter().max().copied()?,
        ))
    } else {
        None
    }
}
