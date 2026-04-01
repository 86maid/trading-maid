use crate::series::Series;

/// Calculates the Simple Moving Average (SMA) for a given series of prices and a specified length.
/// Returns the SMA value or NaN if the length is zero or if there are not enough data points in the series.
pub fn ma(series: &Series, length: usize) -> f64 {
    if length == 0 || series.len() < length {
        return f64::NAN;
    }

    series.iter().take(length).sum::<f64>() / length as f64
}

/// Calculates the Exponential Moving Average (EMA) for a given series of prices and a specified length.
/// Returns the EMA value or NaN if the length is zero or if there are not enough data points in the series.
pub fn ema(series: &Series, length: usize) -> f64 {
    if length == 0 || series.len() < length {
        return f64::NAN;
    }

    let mut ema_cache = EMACache::new(length);

    for &price in series.iter().rev() {
        ema_cache.update(price);
    }

    ema_cache.value()
}

/// Calculates the Relative Strength Index (RSI) for a given series of prices and a specified length.
/// Returns the RSI value or NaN if the length is zero or if there are not enough data points in the series.
pub fn rsi(series: &Series, length: usize) -> f64 {
    if length == 0 || series.len() < length + 1 {
        return f64::NAN;
    }

    let mut rsi_cache = RSICache::new(length);

    for &price in series.iter().rev() {
        rsi_cache.update(price);
    }

    rsi_cache.value()
}

/// Calculates the Commodity Channel Index (CCI) for given high, low, and close price series and a specified length.
/// Returns the CCI value or NaN if the length is zero or if there are not enough data points in any of the series.
pub fn cci(high: &Series, low: &Series, close: &Series, length: usize) -> f64 {
    if length == 0 || high.len() < length || low.len() < length || close.len() < length {
        return f64::NAN;
    }

    let typical_prices: Vec<f64> = high
        .iter()
        .zip(low.iter())
        .zip(close.iter())
        .take(length)
        .map(|((h, l), c)| (h + l + c) / 3.0)
        .collect();

    let sma_tp = typical_prices.iter().sum::<f64>() / length as f64;
    let mean_deviation = typical_prices
        .iter()
        .map(|tp| (tp - sma_tp).abs())
        .sum::<f64>()
        / length as f64;

    if mean_deviation == 0.0 {
        0.0
    } else {
        (typical_prices[0] - sma_tp) / (0.015 * mean_deviation)
    }
}

/// Calculates the MACD, Signal Line, and Histogram values for a given series of prices.
/// Returns a tuple containing the MACD value, Signal Line value, and Histogram value.
pub fn macd(
    series: &Series,
    fast_length: usize,
    slow_length: usize,
    signal_length: usize,
) -> (f64, f64, f64) {
    if fast_length == 0 || slow_length == 0 || signal_length == 0 || fast_length >= slow_length {
        return (f64::NAN, f64::NAN, f64::NAN);
    }

    let mut fast_ema = EMACache::new(fast_length);
    let mut slow_ema = EMACache::new(slow_length);
    let mut signal_ema = EMACache::new(signal_length);

    for &price in series.iter().rev() {
        let fast = fast_ema.update(price);
        let slow = slow_ema.update(price);

        if !fast.is_nan() && !slow.is_nan() {
            let macd_val = fast - slow;
            signal_ema.update(macd_val);
        }
    }

    let macd_val = fast_ema.value() - slow_ema.value();
    let signal = signal_ema.value();
    let histogram = if signal.is_nan() {
        f64::NAN
    } else {
        macd_val - signal
    };

    (macd_val, signal, histogram)
}

/// Calculates the highest and lowest values in a given series of prices for a specified length.
/// Returns the highest and lowest values or NaN if the length is zero or if there are
pub fn highest(series: &Series, length: usize) -> f64 {
    if length == 0 || series.len() < length {
        return f64::NAN;
    }

    series
        .iter()
        .take(length)
        .copied()
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(f64::NAN)
}

/// Calculates the lowest value in a given series of prices for a specified length.
/// Returns the lowest value or NaN if the length is zero or if there are not enough data points in the series.
pub fn lowest(series: &Series, length: usize) -> f64 {
    if length == 0 || series.len() < length {
        return f64::NAN;
    }

    series
        .iter()
        .take(length)
        .copied()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(f64::NAN)
}

pub struct EMACache {
    length: usize,
    multiplier: f64,
    current_ema: f64,
    count: usize,
    sum: f64,
}

impl EMACache {
    pub fn new(length: usize) -> Self {
        let multiplier = 2.0 / (length as f64 + 1.0);

        EMACache {
            length,
            multiplier,
            current_ema: 0.0,
            count: 0,
            sum: 0.0,
        }
    }

    pub fn with_ema(length: usize, ema: f64) -> Self {
        let multiplier = 2.0 / (length as f64 + 1.0);

        EMACache {
            length,
            multiplier,
            current_ema: ema,
            count: usize::MAX,
            sum: 0.0,
        }
    }

    pub fn update(&mut self, price: impl Into<f64>) -> f64 {
        let price = price.into();

        self.count = self.count.saturating_add(1);

        if self.count < self.length {
            self.sum += price;
            f64::NAN
        } else if self.count == self.length {
            self.sum += price;
            self.current_ema = self.sum / self.length as f64;
            self.current_ema
        } else {
            self.current_ema = price * self.multiplier + self.current_ema * (1.0 - self.multiplier);
            self.current_ema
        }
    }

    pub fn value(&self) -> f64 {
        if self.count != usize::MAX && self.count < self.length {
            f64::NAN
        } else {
            self.current_ema
        }
    }

    pub fn reset(&mut self) {
        self.count = 0;
        self.sum = 0.0;
        self.current_ema = 0.0;
    }
}

pub struct RSICache {
    length: usize,
    count: usize,
    last_price: f64,
    has_last_price: bool,
    sum_gain: f64,
    sum_loss: f64,
    avg_gain: f64,
    avg_loss: f64,
}

impl RSICache {
    pub fn new(length: usize) -> Self {
        RSICache {
            length,
            count: 0,
            last_price: 0.0,
            has_last_price: false,
            sum_gain: 0.0,
            sum_loss: 0.0,
            avg_gain: 0.0,
            avg_loss: 0.0,
        }
    }

    pub fn update(&mut self, price: impl Into<f64>) -> f64 {
        let price = price.into();

        if self.length == 0 {
            return f64::NAN;
        }

        if !self.has_last_price {
            self.last_price = price;
            self.has_last_price = true;
            return f64::NAN;
        }

        let delta = price - self.last_price;
        self.last_price = price;

        let gain = delta.max(0.0);
        let loss = (-delta).max(0.0);

        self.count = self.count.saturating_add(1);

        if self.count < self.length {
            self.sum_gain += gain;
            self.sum_loss += loss;
            return f64::NAN;
        }

        if self.count == self.length {
            self.sum_gain += gain;
            self.sum_loss += loss;
            self.avg_gain = self.sum_gain / self.length as f64;
            self.avg_loss = self.sum_loss / self.length as f64;
            return self.value();
        }

        self.avg_gain = (self.avg_gain * (self.length as f64 - 1.0) + gain) / self.length as f64;
        self.avg_loss = (self.avg_loss * (self.length as f64 - 1.0) + loss) / self.length as f64;
        self.value()
    }

    pub fn value(&self) -> f64 {
        if self.count < self.length {
            return f64::NAN;
        }

        if self.avg_loss == 0.0 {
            return if self.avg_gain == 0.0 { 50.0 } else { 100.0 };
        }

        let rs = self.avg_gain / self.avg_loss;
        100.0 - 100.0 / (1.0 + rs)
    }
}
