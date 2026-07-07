use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
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

        if let (Some(fast), Some(slow)) = (fast, slow) {
            signal_ema.update(fast - slow);
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

    pub fn with_ema<T>(length: usize, ema: T) -> Self
    where
        T: TryInto<Decimal>,
        <T as TryInto<Decimal>>::Error: std::fmt::Debug,
    {
        let multiplier = dec!(2) / (Decimal::from(length) + dec!(1));

        EMACache {
            length,
            multiplier,
            current_ema: Some(ema.try_into().expect("failed to convert EMA value")),
            count: usize::MAX,
            sum: dec!(0),
        }
    }

    pub fn update<T>(&mut self, price: T) -> Option<Decimal>
    where
        T: TryInto<Decimal>,
        <T as TryInto<Decimal>>::Error: std::fmt::Debug,
    {
        let price = price
            .try_into()
            .expect("failed to convert price to Decimal");

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

/// Calculates the Average True Range (ATR) for a given period.
///
/// Returns `Some(ATR)` or `None` if the length is zero or insufficient data.
pub fn atr(high: &Series, low: &Series, close: &Series, length: usize) -> Option<Decimal> {
    if length == 0 || high.len() < length + 1 || low.len() < length + 1 || close.len() < length + 1
    {
        return None;
    }

    let sum = (0..length)
        .map(|i| {
            let h = high[i];
            let l = low[i];
            let prev_c = close[i + 1];

            (h - l).max((h - prev_c).abs()).max((l - prev_c).abs())
        })
        .sum::<Decimal>();

    Some(sum / Decimal::from(length))
}

/// Calculates Bollinger Bands for a given series.
///
/// Returns `(middle, upper, lower)` where middle is the SMA, upper and lower are
/// `multiplier` standard deviations away from the middle.
/// Returns `None` for each band if insufficient data.
pub fn bollinger<T>(
    series: &Series,
    length: usize,
    multiplier: T,
) -> (Option<Decimal>, Option<Decimal>, Option<Decimal>)
where
    T: TryInto<Decimal>,
    <T as TryInto<Decimal>>::Error: std::fmt::Debug,
{
    let multiplier = multiplier
        .try_into()
        .expect("failed to convert multiplier to Decimal");

    if length == 0 || series.len() < length {
        return (None, None, None);
    }

    let Some(middle) = ma(series, length) else {
        return (None, None, None);
    };

    let prices: Vec<Decimal> = series.iter().take(length).copied().collect();

    let variance: Decimal = prices
        .iter()
        .map(|p| {
            let diff = p - middle;
            diff * diff
        })
        .sum::<Decimal>()
        / Decimal::from(length);

    let Some(std_dev) = variance.sqrt() else {
        return (None, None, None);
    };

    let upper = middle + multiplier * std_dev;
    let lower = middle - multiplier * std_dev;

    (Some(middle), Some(upper), Some(lower))
}

/// Standard pivot points.
///
/// Returns `(pp, r1, r2, r3, s1, s2, s3)` where:
/// - `pp`   = (high + low + close) / 3
/// - `r1`   = 2 * pp - low
/// - `r2`   = pp + (high - low)
/// - `r3`   = high + 2 * (pp - low)
/// - `s1`   = 2 * pp - high
/// - `s2`   = pp - (high - low)
/// - `s3`   = low - 2 * (high - pp)
///
/// Returns `None` for all values if any required series has fewer than 1 element.
pub fn pivot_point(
    high: &Series,
    low: &Series,
    close: &Series,
) -> (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
) {
    if high.len() < 1 || low.len() < 1 || close.len() < 1 {
        return (None, None, None, None, None, None, None);
    }

    let h = high[0];
    let l = low[0];
    let c = close[0];
    let range = h - l;

    let pp = (h + l + c) / dec!(3);
    let r1 = dec!(2) * pp - l;
    let r2 = pp + range;
    let r3 = h + dec!(2) * (pp - l);
    let s1 = dec!(2) * pp - h;
    let s2 = pp - range;
    let s3 = l - dec!(2) * (h - pp);

    (
        Some(pp),
        Some(r1),
        Some(r2),
        Some(r3),
        Some(s1),
        Some(s2),
        Some(s3),
    )
}

/// Fibonacci pivot points.
///
/// Returns `(pp, r1, r2, r3, s1, s2, s3)` where:
/// - `pp`   = (high + low + close) / 3
/// - `r1`   = pp + 0.382 * range
/// - `r2`   = pp + 0.618 * range
/// - `r3`   = pp + 1.000 * range
/// - `s1`   = pp - 0.382 * range
/// - `s2`   = pp - 0.618 * range
/// - `s3`   = pp - 1.000 * range
///
/// Returns `None` for all values if any required series has fewer than 1 element.
pub fn fib_pivot_point(
    high: &Series,
    low: &Series,
    close: &Series,
) -> (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
) {
    if high.len() < 1 || low.len() < 1 || close.len() < 1 {
        return (None, None, None, None, None, None, None);
    }

    let h = high[0];
    let l = low[0];
    let c = close[0];
    let range = h - l;

    let pp = (h + l + c) / dec!(3);
    let r1 = pp + dec!(0.382) * range;
    let r2 = pp + dec!(0.618) * range;
    let r3 = pp + range;
    let s1 = pp - dec!(0.382) * range;
    let s2 = pp - dec!(0.618) * range;
    let s3 = pp - range;

    (
        Some(pp),
        Some(r1),
        Some(r2),
        Some(r3),
        Some(s1),
        Some(s2),
        Some(s3),
    )
}

/// Camarilla pivot points.
///
/// Returns `(pp, r1, r2, r3, r4, s1, s2, s3, s4)` where:
/// - `pp`   = (high + low + close) / 3
/// - `r1`   = close + range * 1.1 / 12
/// - `r2`   = close + range * 1.1 / 6
/// - `r3`   = close + range * 1.1 / 4
/// - `r4`   = close + range * 1.1 / 2
/// - `s1`   = close - range * 1.1 / 12
/// - `s2`   = close - range * 1.1 / 6
/// - `s3`   = close - range * 1.1 / 4
/// - `s4`   = close - range * 1.1 / 2
///
/// Returns `None` for all values if any required series has fewer than 1 element.
pub fn camarilla_pivot_point(
    high: &Series,
    low: &Series,
    close: &Series,
) -> (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
) {
    if high.len() < 1 || low.len() < 1 || close.len() < 1 {
        return (None, None, None, None, None, None, None, None, None);
    }

    let h = high[0];
    let l = low[0];
    let c = close[0];
    let range = h - l;

    let pp = (h + l + c) / dec!(3);
    let factor = range * dec!(1.1);

    let r1 = c + factor / dec!(12);
    let r2 = c + factor / dec!(6);
    let r3 = c + factor / dec!(4);
    let r4 = c + factor / dec!(2);

    let s1 = c - factor / dec!(12);
    let s2 = c - factor / dec!(6);
    let s3 = c - factor / dec!(4);
    let s4 = c - factor / dec!(2);

    (
        Some(pp),
        Some(r1),
        Some(r2),
        Some(r3),
        Some(r4),
        Some(s1),
        Some(s2),
        Some(s3),
        Some(s4),
    )
}

/// Woodie pivot points.
///
/// Returns `(pp, r1, r2, s1, s2)` where:
/// - `pp`   = (high + low + 2 * close) / 4
/// - `r1`   = 2 * pp - low
/// - `r2`   = pp + (high - low)
/// - `s1`   = 2 * pp - high
/// - `s2`   = pp - (high - low)
///
/// Returns `None` for all values if any required series has fewer than 1 element.
pub fn woodie_pivot_point(
    high: &Series,
    low: &Series,
    close: &Series,
) -> (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
) {
    if high.len() < 1 || low.len() < 1 || close.len() < 1 {
        return (None, None, None, None, None);
    }

    let h = high[0];
    let l = low[0];
    let c = close[0];
    let range = h - l;

    let pp = (h + l + dec!(2) * c) / dec!(4);
    let r1 = dec!(2) * pp - l;
    let r2 = pp + range;
    let s1 = dec!(2) * pp - h;
    let s2 = pp - range;

    (Some(pp), Some(r1), Some(r2), Some(s1), Some(s2))
}

/// Detects when `fast` crosses **above** `slow` (golden cross).
///
/// Returns `true` if `fast[0] > slow[0]` and `fast[1] <= slow[1]`.
/// Returns `false` if either series has fewer than 2 elements.
pub fn cross_over(fast: &Series, slow: &Series) -> bool {
    if fast.len() < 2 || slow.len() < 2 {
        return false;
    }

    fast[0] > slow[0] && fast[1] <= slow[1]
}

/// Detects when `fast` crosses **below** `slow` (dead cross).
///
/// Returns `true` if `fast[0] < slow[0]` and `fast[1] >= slow[1]`.
/// Returns `false` if either series has fewer than 2 elements.
pub fn cross_under(fast: &Series, slow: &Series) -> bool {
    if fast.len() < 2 || slow.len() < 2 {
        return false;
    }

    fast[0] < slow[0] && fast[1] >= slow[1]
}
