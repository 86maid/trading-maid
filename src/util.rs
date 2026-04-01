use crate::data::{DataSource, KLine, Level};
use crate::order::{HistoryPosition, HistoryPositionSummary, OrderMessage, Side};
use anyhow::Context;
use anyhow::bail;
use chrono::{
    DateTime, Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc,
    Weekday,
};
use rand::RngExt;
use reqwest::Client;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

/// Downloads and merges K-line data from Binance into a single CSV file
///
/// # Arguments
/// - `format`       - Data format: `symbol/interval[/market_type]`
///   - Market type is optional, defaults to `futures`, can be `spot` or `futures`
///   - Examples: `"BTCUSDT/1m"` or `"ETHUSDT/1h/spot"`
///   - For interval format, see: `Level` enum.
/// - `month_count`  - Number of months to download, 0 means all available data (up to 120 months)
///
/// # Returns
/// - On success, returns the path to the merged CSV file
/// - Data storage location: `~/.trading-maid/<symbol>/<interval>.csv`
///
/// # Data Source
/// https://data.binance.vision
///
/// # Processing Pipeline
/// 1. Parse format string to extract symbol, interval, and market type
/// 2. Generate list of months to download (defaults to last 120 months)
/// 3. Check local cache for each month, download from Binance if missing
/// 4. Merge all monthly CSV files, removing duplicate headers
/// 5. Return path to merged file
///
/// # Note
/// During merging, only the first 6 columns are kept (timestamp, open, high, low, close, volume)
pub async fn get_or_download(format: &str, month_count: u32) -> anyhow::Result<PathBuf> {
    fn parse_symbol(input: &str) -> anyhow::Result<(&str, &str, &str)> {
        let part_list: Vec<&str> = input.split('/').collect();

        match part_list.as_slice() {
            [symbol, interval] => Ok((symbol, interval, "futures")),
            [symbol, interval, market_type]
                if *market_type == "spot" || *market_type == "futures" =>
            {
                Ok((symbol, interval, market_type))
            }
            _ => bail!("invalid format: '{}'", input),
        }
    }

    fn generate_month_list(count: u32) -> Vec<String> {
        let mut month_list = Vec::with_capacity(count as usize);

        let mut current_date = {
            let now = Utc::now();
            let previous_month = if now.month() > 1 { now.month() - 1 } else { 12 };
            let year = if now.month() == 1 {
                now.year() - 1
            } else {
                now.year()
            };

            chrono::NaiveDate::from_ymd_opt(year, previous_month, 1).unwrap()
        };

        for _ in 0..count {
            month_list.push(current_date.format("%Y-%m").to_string());
            current_date = current_date.pred_opt().unwrap().with_day(1).unwrap();
        }

        month_list.reverse();

        month_list
    }

    fn build_download_url(
        base_url: &str,
        symbol: &str,
        interval: &str,
        year_month: &str,
        market_type: &str,
    ) -> String {
        let url_prefix = if market_type == "spot" {
            "spot/monthly"
        } else {
            "futures/um/monthly"
        };

        format!(
            "{}/{}/klines/{}/{}/{}-{}-{}.zip",
            base_url, url_prefix, symbol, interval, symbol, interval, year_month
        )
    }

    async fn download_monthly_data(
        http_client: &Client,
        url: &str,
    ) -> anyhow::Result<Option<String>> {
        #[cfg(debug_assertions)]
        println!("download: {}", url);

        let response = http_client.get(url).send().await?;

        if response.status() == 404 {
            return Ok(None);
        }

        response.error_for_status_ref()?;

        let response_bytes = response.bytes().await?;

        let mut zip_archive = ZipArchive::new(std::io::Cursor::new(response_bytes))?;

        let csv_filename = zip_archive
            .file_names()
            .find(|filename| filename.ends_with(".csv"))
            .context("no csv in zip")?
            .to_owned();

        let mut csv_content = String::new();

        zip_archive
            .by_name(&csv_filename)?
            .read_to_string(&mut csv_content)?;

        Ok(Some(csv_content))
    }

    let (base_symbol, interval, market_type) = parse_symbol(format)?;

    let base_data_directory = dirs::home_dir()
        .map(|home_directory| home_directory.join(".trading-maid"))
        .context("can not find data dir")?;

    let symbol_directory = base_data_directory.join(base_symbol);
    let monthly_directory = symbol_directory.join(interval);
    let merged_file_path = symbol_directory.join(format!("{}.csv", interval));
    let marged_lock_path = symbol_directory.join(format!("{}.lock", interval));

    fs::create_dir_all(&monthly_directory).await?;

    let month_list = if month_count == 0 {
        generate_month_list(120)
    } else {
        generate_month_list(month_count)
    };

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    const BASE_URL: &str = "https://data.binance.vision/data";

    for v in &month_list {
        let monthly_file_path = monthly_directory.join(format!("{}.csv", v));

        if monthly_file_path.exists() {
            continue;
        }

        let download_url = build_download_url(BASE_URL, base_symbol, interval, v, market_type);

        match download_monthly_data(&http_client, &download_url).await {
            Ok(Some(v)) => {
                fs::write(&monthly_file_path, v.as_bytes()).await?;
                tokio::fs::write(&marged_lock_path, "").await?;
            }
            Ok(None) => {
                #[cfg(debug_assertions)]
                println!("{} not available (404)", v)
            }
            Err(error) => {
                bail!(
                    "failed to download {}: {}",
                    v,
                    error.to_string().to_lowercase()
                );
            }
        }
    }

    async fn merge_monthly_files(
        monthly_directory: &Path,
        output_file_path: &Path,
        marged_lock_path: &Path,
    ) -> anyhow::Result<()> {
        if !marged_lock_path.exists() && output_file_path.exists() {
            return Ok(());
        }

        let mut csv_file_list = Vec::new();

        let mut directory_reader = fs::read_dir(monthly_directory).await?;

        while let Some(v) = directory_reader.next_entry().await? {
            let file_path = v.path();

            if file_path
                .extension()
                .is_some_and(|extension| extension == "csv")
            {
                csv_file_list.push(file_path);
            }
        }

        csv_file_list.sort();

        if csv_file_list.is_empty() {
            bail!(
                "no monthly files found in {:?}",
                monthly_directory.to_string_lossy().to_lowercase()
            );
        }

        let mut output_file = fs::File::create(output_file_path).await?;

        for v in csv_file_list.iter() {
            let file_content = fs::read_to_string(v).await?;

            let mut content_lines = file_content.lines();

            _ = content_lines.next();

            for v in content_lines {
                let Some((pos, _)) = v.match_indices(',').nth(5) else {
                    bail!("parse csv error: less than 6 commas");
                };

                output_file.write_all(&v.as_bytes()[..pos]).await?;
                output_file.write_all(b"\n").await?;
            }
        }

        if marged_lock_path.exists() {
            std::fs::remove_file(marged_lock_path)?;
        }

        Ok(())
    }

    merge_monthly_files(&monthly_directory, &merged_file_path, &marged_lock_path).await?;

    Ok(merged_file_path)
}

/// Calculates the liquidation price for a given position based on leverage, maintenance margin,
/// position side (buy or sell), the current price, position quantity, and available margin.
///
/// The liquidation price is the price at which the position will be closed to prevent further loss,
/// given the provided leverage, maintenance margin, and current account margin.
///
/// # Arguments
///
/// - `leverage`: The leverage used in the position. It is a multiplier that amplifies the exposure to the market.
/// - `maintenance`: The maintenance margin requirement, expressed as a percentage.
/// - `side`: The side of the position, either `Side::Buy` or `Side::Sell`. Determines the calculation logic.
/// - `price`: The current market price of the asset.
/// - `quantity`: The amount of the asset in the position.
/// - `margin`: The current margin available in the account to support the position.
///
/// # Returns
///
/// - The liquidation price as a floating-point number (`f64`). This price represents the point at which the position
///   will be liquidated if the market moves unfavorably, considering the leverage and maintenance margin.
///
/// # Calculation Logic
///
/// 1. The initial margin is calculated by dividing the position's value (`price * quantity`) by the leverage.
/// 2. The additional margin (`append_margin`) is the difference between the available margin and the initial margin.
/// 3. Depending on whether the position is a `Buy` or `Sell`:
///    - For a `Buy`, the liquidation price is adjusted downwards based on the initial margin rate and the maintenance margin.
///    - For a `Sell`, the liquidation price is adjusted upwards similarly, considering the maintenance margin.
pub fn calc_liquidation_price(
    leverage: u32,
    maintenance: f64,
    side: Side,
    price: f64,
    quantity: f64,
    margin: f64,
) -> f64 {
    let initial_margin_rate = 1.0 / leverage as f64;
    let initial_margin = calc_initial_margin(price, quantity, leverage);
    let append_margin = margin - initial_margin;

    if side == Side::Buy {
        price * (1.0 - initial_margin_rate + maintenance) - ((append_margin) / quantity)
    } else {
        price * (1.0 + initial_margin_rate - maintenance) + ((append_margin) / quantity)
    }
}

/// Calculates the initial margin required for a position based on its price, quantity, and leverage.
///
/// # Arguments
///
/// - `price`: The price of the asset.
/// - `quantity`: The amount of the asset in the position.
/// - `leverage`: The leverage used in the position.
///
/// # Returns
///
/// The initial margin as a floating-point number (`f64`).
pub fn calc_initial_margin(price: f64, quantity: f64, leverage: u32) -> f64 {
    price * quantity / leverage as f64
}

/// Calculates the slippage-adjusted price range based on the current price and slippage percentage.
///
/// # Arguments
///
/// - `price`: The current price of the asset.
/// - `slippage`: The slippage percentage.
///
/// # Returns
///
/// A tuple containing the minimum and maximum prices after accounting for slippage.
pub fn calc_slippage_price(price: f64, slippage: f64) -> (f64, f64) {
    let factor = slippage / 100.0;
    let min_price = price * (1.0 - factor);
    let max_price = price * (1.0 + factor);

    (min_price, max_price)
}

/// Generates a random price within the slippage-adjusted price range, snapped to the nearest tick size.
pub fn get_random_slippage_price(price: f64, slippage: f64, tick_size: f64) -> Option<f64> {
    let (min, max) = calc_slippage_price(price, slippage);

    if tick_size <= 0.0 || min > max {
        return None;
    }

    let min = (min / tick_size).round() as i64;
    let max = (max / tick_size).round() as i64;

    if min > max {
        return None;
    }

    Some(rand::rng().random_range(min..=max) as f64 * tick_size)
}

/// Checks if a given price value is aligned with the specified tick size, considering floating-point precision issues.
pub fn is_tick_aligned(value: f64, tick_size: f64) -> bool {
    if !value.is_finite() || !tick_size.is_finite() || tick_size <= 0.0 {
        return false;
    }

    let steps = value / tick_size;
    let tolerance = 1e-9_f64.max(f64::EPSILON * steps.abs() * 16.0);

    (steps - steps.round()).abs() <= tolerance
}

pub trait TickSnap {
    fn snap_to_tick(self, tick_size: f64) -> f64;
    fn snap_eq(self, other: f64, tick_size: f64) -> bool;
    fn snap_lt(self, other: f64, tick_size: f64) -> bool;
    fn snap_gt(self, other: f64, tick_size: f64) -> bool;
    fn snap_le(self, other: f64, tick_size: f64) -> bool;
    fn snap_ge(self, other: f64, tick_size: f64) -> bool;
    fn is_zero(self, tick_size: f64) -> bool;
}

impl TickSnap for f64 {
    fn is_zero(self, tick_size: f64) -> bool {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return self == 0.0;
        }

        self.abs() <= tick_size / 2.0
    }

    fn snap_to_tick(self, tick_size: f64) -> f64 {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return self;
        }

        (self / tick_size).round() * tick_size
    }

    fn snap_eq(self, other: f64, tick_size: f64) -> bool {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return self == other;
        }

        (self - other).abs() <= tick_size / 2.0
    }

    fn snap_lt(self, other: f64, tick_size: f64) -> bool {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return self < other;
        }

        self < other - tick_size / 2.0
    }

    fn snap_gt(self, other: f64, tick_size: f64) -> bool {
        if !tick_size.is_finite() || tick_size <= 0.0 {
            return self > other;
        }

        self > other + tick_size / 2.0
    }

    fn snap_le(self, other: f64, tick_size: f64) -> bool {
        self.snap_lt(other, tick_size) || self.snap_eq(other, tick_size)
    }

    fn snap_ge(self, other: f64, tick_size: f64) -> bool {
        self.snap_gt(other, tick_size) || self.snap_eq(other, tick_size)
    }
}

/// Converts a Unix timestamp in milliseconds to a formatted date-time string
/// in the **system local timezone**.
/// Returns the formatted string in `%Y-%m-%d %H:%M:%S%.f` format.
pub fn t2s(time: impl Into<u64>) -> String {
    Local
        .timestamp_millis_opt(time.into() as i64)
        .single()
        .unwrap_or_default()
        .format("%Y/%m/%d %H:%M:%S%.f")
        .to_string()
}

/// Converts a formatted date-time string (in local timezone) to Unix timestamp in milliseconds.
/// Input format: `%Y-%m-%d %H:%M:%S` or `%Y-%m-%d %H:%M:%S%.f`
/// Returns timestamp in milliseconds. Returns `0` if parsing fails.
pub fn s2t(time: impl AsRef<str>) -> u64 {
    NaiveDateTime::parse_from_str(time.as_ref(), "%Y/%m/%d %H:%M:%S%.f")
        .map(|v| {
            Local
                .from_local_datetime(&v)
                .single()
                .unwrap_or_default()
                .timestamp_millis() as u64
        })
        .unwrap_or(0)
}

/// Converts a formatted date-time string (in UTC timezone) to Unix timestamp in milliseconds.
/// Input format: `%Y-%m-%d %H:%M:%S` or `%Y-%m-%d %H:%M:%S%.f`
/// Returns timestamp in milliseconds. Returns `0` if parsing fails.
pub fn s2t_utc(time: impl AsRef<str>) -> u64 {
    NaiveDateTime::parse_from_str(time.as_ref(), "%Y/%m/%d %H:%M:%S%.f")
        .map(|v| v.and_utc().timestamp_millis() as u64)
        .unwrap_or(0)
}

/// Converts a Unix timestamp in milliseconds to a formatted date-time string in UTC timezone.
/// Returns the formatted string in `%Y-%m-%d %H:%M:%S%.f` format.
pub fn t2s_utc(time: impl Into<u64>) -> String {
    DateTime::<Utc>::from_timestamp_millis(time.into() as i64)
        .unwrap_or_default()
        .format("%Y/%m/%d %H:%M:%S%.f")
        .to_string()
}

/// Given an arbitrary time `time`, calculate the end time of a larger time level `max_level`
/// within the context of a smaller time level `min_level`.
/// If `max_level` and `min_level` are the same, return the current time.
/// If `max_level` is not a valid sampling target for `min_level`, return 0.
///
/// # Parameters
/// - `time`: The timestamp of the current k-line.
/// - `min_level`: The minimum time level to calculate the end time for.
/// - `max_level`: The time level to which the `time` should be adjusted.
///
/// # Returns
/// The end timestamp for the given `time` at the specified `min_level`, or `0` if the levels are incompatible.
///
/// # Example:
/// Let’s say the `min_level` is `Level::Minute1`, and `max_level` is `Level::Hour1`.
/// Given `time = 1678845600000` (representing `2023-03-15 10:00:00`).
/// The function should return `1678849599000`, which represents the timestamp for `2023-03-15 10:59:00`.
pub fn get_last_time(time: u64, min_level: Level, max_level: Level) -> anyhow::Result<u64> {
    if !min_level.is_valid_sampling_target(max_level) {
        bail!(
            "invalid sampling target level: min_level: {}, max_level: {}",
            min_level,
            max_level
        );
    }

    if max_level == min_level {
        return Ok(time);
    }

    Ok(get_time_range(get_time_range(time, max_level)?.1 - 1, min_level)?.0)
}

/// Calculates the start time of the current k-line time period and the start time of the next time period
/// based on the specified time level.
///
/// # Parameters
/// - `time`: The timestamp of the current k-line.
/// - `level`: The time level to convert to.
///
/// # Returns
/// A tuple containing:
/// - The start timestamp of the current k-line time period.
/// - The start timestamp of the next k-line time period.
pub fn get_time_range(time: u64, level: Level) -> anyhow::Result<(u64, u64)> {
    match level {
        Level::Minute1 => {
            let start = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let next = start + Duration::minutes(1);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Minute3 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::minutes((dt.minute() as i32 % 3) as i64);
            let next = start + Duration::minutes(3);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Minute5 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::minutes((dt.minute() as i32 % 5) as i64);
            let next = start + Duration::minutes(5);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Minute15 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::minutes((dt.minute() as i32 % 15) as i64);
            let next = start + Duration::minutes(15);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Minute30 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::minutes((dt.minute() as i32 % 30) as i64);
            let next = start + Duration::minutes(30);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Hour1 => {
            let start = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_minute(0)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let next = start + Duration::hours(1);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Hour2 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_minute(0)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::hours((dt.hour() as i32 % 2) as i64);
            let next = start + Duration::hours(2);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Hour4 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_minute(0)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::hours((dt.hour() as i32 % 4) as i64);
            let next = start + Duration::hours(4);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Hour6 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_minute(0)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::hours((dt.hour() as i32 % 6) as i64);
            let next = start + Duration::hours(6);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Hour12 => {
            let dt = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .with_minute(0)
                .context("get_time_range")?
                .with_second(0)
                .context("get_time_range")?
                .with_nanosecond(0)
                .context("get_time_range")?;

            let start = dt - Duration::hours((dt.hour() as i32 % 12) as i64);
            let next = start + Duration::hours(12);

            Ok((
                start.timestamp_millis() as u64,
                next.timestamp_millis() as u64,
            ))
        }
        Level::Day1 => {
            let start = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .context("get_time_range")?;

            let next = start + Duration::days(1);

            Ok((
                start.and_utc().timestamp_millis() as u64,
                next.and_utc().timestamp_millis() as u64,
            ))
        }
        Level::Day3 => {
            let count = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .context("get_time_range")?
                .num_days_from_ce();

            let start = NaiveDate::from_num_days_from_ce_opt(count / 3 * 3)
                .context("get_time_range")?
                .and_hms_opt(0, 0, 0)
                .context("get_time_range")?;

            let next = start + Duration::days(3);

            Ok((
                start.and_utc().timestamp_millis() as u64,
                next.and_utc().timestamp_millis() as u64,
            ))
        }
        Level::Week1 => {
            let start = DateTime::from_timestamp_millis(time as i64)
                .context("get_time_range")?
                .date_naive()
                .week(Weekday::Mon)
                .first_day()
                .and_hms_opt(0, 0, 0)
                .context("get_time_range")?;

            let next = start + Duration::weeks(1);

            Ok((
                start.and_utc().timestamp_millis() as u64,
                next.and_utc().timestamp_millis() as u64,
            ))
        }
        Level::Month1 => {
            let start = Datelike::with_day(
                &DateTime::from_timestamp_millis(time as i64)
                    .context("get_time_range")?
                    .date_naive(),
                1,
            )
            .context("get_time_range")?
            .and_hms_opt(0, 0, 0)
            .context("get_time_range")?;

            let next = start + Months::new(1);

            Ok((
                start.and_utc().timestamp_millis() as u64,
                next.and_utc().timestamp_millis() as u64,
            ))
        }
    }
}

/// Aggregates a lower time-level k-line array into a higher time-level k-line array.
///
/// # Note
/// The target time level must be an integer multiple of the original time level, otherwise the results may be inaccurate.
/// The function does not ensure that the first and last k-lines in the resulting array will align perfectly with the specified time level;
/// they may be partial k-lines if the input data does not cover a complete period for the given level.
///
/// # Parameters
/// - `array`: The input k-line array.
/// - `level`: The target time level.
///
/// # Returns
/// The aggregated k-line array.
pub fn resample(array: &[KLine], level: Level) -> anyhow::Result<Vec<KLine>> {
    let mut result = Vec::new();

    if array.is_empty() {
        return Ok(result);
    }

    let mut start_index = 0;

    while start_index < array.len() {
        let start_k = array[start_index];

        let (start_time, next_time) = get_time_range(start_k.time, level)?;

        let next_index = array[start_index..]
            .iter()
            .position(|v| v.time >= next_time)
            .map(|v| start_index + v)
            .unwrap_or(array.len());

        let last_k = array[..next_index].last().unwrap();

        let mut result_k = KLine {
            time: start_time,
            open: start_k.open,
            high: start_k.high,
            low: start_k.low,
            close: last_k.close,
            volume: 0.0,
        };

        for v in &array[start_index..next_index] {
            result_k.high = result_k.high.max(v.high);
            result_k.low = result_k.low.min(v.low);
            result_k.volume += v.volume;
        }

        result.push(result_k);

        start_index = next_index;
    }

    Ok(result)
}

/// Resamples k-line data from a file into multiple different time levels and writes them to new files.
///
/// This function reads a k-line data file, determines the current time level from the file name,
/// and generates aggregated k-line data for each of the following higher time levels. The resampled data is written
/// to new files, with each file corresponding to one of the higher time levels.
///
/// # Parameters
/// - `path`: The path to the input k-line data file. The file name should include the time level (e.g., `BTC-USDT-SWAP-Minute1.json`).
///
/// # Returns
/// Returns a result indicating success or failure (`anyhow::Result<()>`).
pub fn resample_file(path: impl AsRef<Path>) -> anyhow::Result<()> {
    let path = path.as_ref();

    let level = Level::from_str(
        path.file_name()
            .context("resample_file")?
            .to_string_lossy()
            .split('-')
            .next_back()
            .context("resample_file")?
            .split('.')
            .next()
            .context("resample_file")?,
    )?;

    let file_stem = path
        .file_stem()
        .context("resample_file")?
        .to_string_lossy()
        .replace(&format!("-{}", level), "");

    let extension = path
        .extension()
        .context("resample_file")?
        .to_string_lossy()
        .to_string();

    let ds = DataSource::from_file(path)?;

    let level_list = [
        Level::Minute1,
        Level::Minute3,
        Level::Minute5,
        Level::Minute15,
        Level::Minute30,
        Level::Hour1,
        Level::Hour2,
        Level::Hour4,
        Level::Hour6,
        Level::Hour12,
        Level::Day1,
        Level::Day3,
        Level::Week1,
        Level::Month1,
    ];

    for v in level_list.into_iter().filter(|&v| v > level) {
        ds.resample(v)?
            .write_file(path.with_file_name(format!("{}-{}.{}", file_stem, v, extension)))?;
    }

    Ok(())
}

/// Converts the provided data sources, history positions, and history orders into an HTML string that can be rendered in a web browser.
pub fn to_html(
    data_source: impl AsRef<[DataSource]>,
    history_position: impl AsRef<[HistoryPosition]>,
    history_order: impl AsRef<[OrderMessage]>,
) -> String {
    let text = format!(
        "dataSourceList={};historyPositionList={};orderList={}",
        &serde_json::to_string(data_source.as_ref()).unwrap(),
        &serde_json::to_string(history_position.as_ref()).unwrap(),
        &serde_json::to_string(history_order.as_ref()).unwrap(),
    );

    include_str!("../web/dist/index.html").replace("{{ code }}", &text)
}

/// Summarizes history positions into the same metrics used by the web summary panel.
pub fn summarize(list: impl AsRef<[HistoryPosition]>) -> HistoryPositionSummary {
    let list = list.as_ref();
    let symbol = list.first().map(|v| v.symbol.clone()).unwrap_or_default();
    let leverage = list.first().map(|v| v.leverage).unwrap_or_default();

    let total_trades = list.len();
    let total_profit = list.iter().map(|v| v.total_profit).sum::<f64>();
    let total_fee = list.iter().map(|v| v.fee).sum::<f64>();
    let win_trades = list.iter().filter(|v| v.total_profit > 0.0).count();
    let loss_trades = list.iter().filter(|v| v.total_profit < 0.0).count();

    let win_rate = if total_trades == 0 {
        0.0
    } else {
        win_trades as f64 / total_trades as f64 * 100.0
    };

    let avg_profit = if total_trades == 0 {
        0.0
    } else {
        total_profit / total_trades as f64
    };

    let net_gross_profit = list
        .iter()
        .filter(|v| v.total_profit > 0.0)
        .map(|v| v.total_profit)
        .sum::<f64>();

    let net_gross_loss_abs = -list
        .iter()
        .filter(|v| v.total_profit < 0.0)
        .map(|v| v.total_profit)
        .sum::<f64>();

    let profit_loss_ratio = if net_gross_loss_abs == 0.0 {
        0.0
    } else {
        net_gross_profit / net_gross_loss_abs
    };

    let gross_profit = list
        .iter()
        .filter(|v| v.profit > 0.0)
        .map(|v| v.profit)
        .sum::<f64>();

    let gross_loss_abs = -list
        .iter()
        .filter(|v| v.profit < 0.0)
        .map(|v| v.profit)
        .sum::<f64>();

    let best_trade = list
        .iter()
        .map(|v| v.total_profit)
        .reduce(f64::max)
        .unwrap_or_default();

    let worst_trade = list
        .iter()
        .map(|v| v.total_profit)
        .reduce(f64::min)
        .unwrap_or_default();

    HistoryPositionSummary {
        symbol,
        leverage,
        total_trades,
        win_rate,
        win_trades,
        loss_trades,
        total_profit,
        profit_loss_ratio,
        net_gross_profit,
        net_gross_loss_abs,
        gross_profit,
        gross_loss_abs,
        total_fee,
        avg_profit,
        best_trade,
        worst_trade,
    }
}

/// Generates an HTML file from the provided data sources, history positions, and history orders, and opens it in the default web browser.
pub fn open_in_browser(
    data_source: impl AsRef<[DataSource]>,
    history_position: impl AsRef<[HistoryPosition]>,
    history_order: impl AsRef<[OrderMessage]>,
) -> anyhow::Result<()> {
    let html_content = to_html(data_source, history_position, history_order);
    let temp_file_path = std::env::temp_dir().join("trading-maid.html");

    std::fs::write(&temp_file_path, html_content)?;
    webbrowser::open(temp_file_path.to_str().context("open_in_browser")?)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_position(total_profit: f64, profit: f64, fee: f64) -> HistoryPosition {
        HistoryPosition {
            symbol: "BTCUSDT".to_string(),
            leverage: 10,
            side: Side::Buy,
            open_avg_price: 100.0,
            close_avg_price: 101.0,
            max_quantity: 1.0,
            close_quantity: 1.0,
            total_profit,
            profit,
            fee,
            open_time: 1,
            close_time: 2,
            log: vec![],
        }
    }

    #[test]
    fn ticksnap_snap_eq_respects_half_tick_boundary() {
        let tick = 0.1;

        assert!(1.04_f64.snap_eq(1.0, tick));
        assert!(1.049_f64.snap_eq(1.0, tick));
        assert!(!1.051_f64.snap_eq(1.0, tick));
    }

    #[test]
    fn ticksnap_snap_lt_gt_use_strict_half_tick_gap() {
        let tick = 0.1;

        assert!(0.949_f64.snap_lt(1.0, tick));
        assert!(!0.95_f64.snap_lt(1.0, tick));

        assert!(1.051_f64.snap_gt(1.0, tick));
        assert!(!1.05_f64.snap_gt(1.0, tick));
    }

    #[test]
    fn ticksnap_snap_le_ge_are_consistent_with_eq_lt_gt() {
        let tick = 0.1;

        assert!(1.0_f64.snap_le(1.0, tick));
        assert!(1.0_f64.snap_ge(1.0, tick));

        assert!(0.94_f64.snap_le(1.0, tick));
        assert!(!0.94_f64.snap_ge(1.0, tick));

        assert!(1.06_f64.snap_ge(1.0, tick));
        assert!(!1.06_f64.snap_le(1.0, tick));
    }

    #[test]
    fn ticksnap_snap_to_tick_rounds_and_invalid_tick_passthrough() {
        assert!((1.24_f64.snap_to_tick(0.1) - 1.2).abs() < 1e-12);
        assert!((1.26_f64.snap_to_tick(0.1) - 1.3).abs() < 1e-12);

        assert_eq!(1.2345_f64.snap_to_tick(0.0), 1.2345);
        assert_eq!(1.2345_f64.snap_to_tick(-0.1), 1.2345);
    }

    #[test]
    fn ticksnap_invalid_tick_falls_back_to_raw_comparison() {
        assert!(0.0_f64.is_zero(0.0));
        assert!(!0.0001_f64.is_zero(0.0));

        assert!(1.0_f64.snap_eq(1.0, 0.0));
        assert!(!1.0_f64.snap_eq(1.0000001, 0.0));

        assert!(0.9_f64.snap_lt(1.0, 0.0));
        assert!(1.1_f64.snap_gt(1.0, 0.0));
    }

    #[test]
    fn is_tick_aligned_works_for_aligned_and_non_aligned_prices() {
        assert!(is_tick_aligned(68000.1, 0.1));
        assert!(is_tick_aligned(68000.1000000001, 0.1));
        assert!(is_tick_aligned(0.1_f64 + 0.2_f64, 0.1));
        assert!(!is_tick_aligned(68000.123, 0.1));
        assert!(!is_tick_aligned(1.0, 0.0));
    }

    #[test]
    fn summarize_matches_web_fields() {
        let data = vec![
            history_position(100.0, 120.0, 20.0),
            history_position(-40.0, -30.0, 10.0),
            history_position(0.0, 5.0, 5.0),
        ];

        let summary = summarize(&data);

        assert_eq!(summary.symbol, "BTCUSDT");
        assert_eq!(summary.leverage, 10);
        assert_eq!(summary.total_trades, 3);
        assert_eq!(summary.win_trades, 1);
        assert_eq!(summary.loss_trades, 1);
        assert!((summary.win_rate - 33.333333).abs() < 1e-6);
        assert!((summary.total_profit - 60.0).abs() < 1e-12);
        assert!((summary.total_fee - 35.0).abs() < 1e-12);
        assert!((summary.avg_profit - 20.0).abs() < 1e-12);
        assert!((summary.net_gross_profit - 100.0).abs() < 1e-12);
        assert!((summary.net_gross_loss_abs - 40.0).abs() < 1e-12);
        assert!((summary.profit_loss_ratio - 2.5).abs() < 1e-12);
        assert!((summary.gross_profit - 125.0).abs() < 1e-12);
        assert!((summary.gross_loss_abs - 30.0).abs() < 1e-12);
        assert!((summary.best_trade - 100.0).abs() < 1e-12);
        assert!((summary.worst_trade + 40.0).abs() < 1e-12);
    }

    #[test]
    fn summarize_empty_returns_zero_values() {
        let summary = summarize([]);

        assert_eq!(summary.symbol, "");
        assert_eq!(summary.leverage, 0);
        assert_eq!(summary.total_trades, 0);
        assert_eq!(summary.win_trades, 0);
        assert_eq!(summary.loss_trades, 0);
        assert_eq!(summary.win_rate, 0.0);
        assert_eq!(summary.total_profit, 0.0);
        assert_eq!(summary.total_fee, 0.0);
        assert_eq!(summary.avg_profit, 0.0);
        assert_eq!(summary.net_gross_profit, 0.0);
        assert_eq!(summary.net_gross_loss_abs, 0.0);
        assert_eq!(summary.profit_loss_ratio, 0.0);
        assert_eq!(summary.gross_profit, 0.0);
        assert_eq!(summary.gross_loss_abs, 0.0);
        assert_eq!(summary.best_trade, 0.0);
        assert_eq!(summary.worst_trade, 0.0);
    }

    macro_rules! assert_time_range {
        ($time_str:expr, $level:expr, $expected_start:expr, $expected_end:expr) => {
            let time = s2t_utc($time_str);
            let (actual_start, actual_end) = get_time_range(time, $level).unwrap();
            assert_eq!(
                (t2s_utc(actual_start), t2s_utc(actual_end)),
                ($expected_start.to_string(), $expected_end.to_string()),
                "time: {}, level: {}",
                $time_str,
                $level
            );
        };
    }

    // ============ 分钟级别测试 ============

    #[test]
    fn test_minute1_basic() {
        assert_time_range!(
            "2024/03/21 10:23:45",
            Level::Minute1,
            "2024/03/21 10:23:00",
            "2024/03/21 10:24:00"
        );
    }

    #[test]
    fn test_minute1_boundary() {
        // 整分钟边界
        assert_time_range!(
            "2024/03/21 10:23:00",
            Level::Minute1,
            "2024/03/21 10:23:00",
            "2024/03/21 10:24:00"
        );
        // 59秒
        assert_time_range!(
            "2024/03/21 10:23:59.999",
            Level::Minute1,
            "2024/03/21 10:23:00",
            "2024/03/21 10:24:00"
        );
    }

    #[test]
    fn test_minute3_various_offsets() {
        // 分钟 % 3 == 0
        assert_time_range!(
            "2024/03/21 10:21:30",
            Level::Minute3,
            "2024/03/21 10:21:00",
            "2024/03/21 10:24:00"
        );
        // 分钟 % 3 == 1
        assert_time_range!(
            "2024/03/21 10:22:15",
            Level::Minute3,
            "2024/03/21 10:21:00",
            "2024/03/21 10:24:00"
        );
        // 分钟 % 3 == 2
        assert_time_range!(
            "2024/03/21 10:23:45",
            Level::Minute3,
            "2024/03/21 10:21:00",
            "2024/03/21 10:24:00"
        );
    }

    #[test]
    fn test_minute5_various_offsets() {
        assert_time_range!(
            "2024/03/21 10:27:30",
            Level::Minute5,
            "2024/03/21 10:25:00",
            "2024/03/21 10:30:00"
        );
        assert_time_range!(
            "2024/03/21 10:30:00",
            Level::Minute5,
            "2024/03/21 10:30:00",
            "2024/03/21 10:35:00"
        );
    }

    #[test]
    fn test_minute15_cross_hour() {
        // 跨小时边界
        assert_time_range!(
            "2024/03/21 10:59:30",
            Level::Minute15,
            "2024/03/21 10:45:00",
            "2024/03/21 11:00:00"
        );
        assert_time_range!(
            "2024/03/21 11:02:00",
            Level::Minute15,
            "2024/03/21 11:00:00",
            "2024/03/21 11:15:00"
        );
    }

    #[test]
    fn test_minute30_basic() {
        assert_time_range!(
            "2024/03/21 10:45:30",
            Level::Minute30,
            "2024/03/21 10:30:00",
            "2024/03/21 11:00:00"
        );
        assert_time_range!(
            "2024/03/21 11:00:00",
            Level::Minute30,
            "2024/03/21 11:00:00",
            "2024/03/21 11:30:00"
        );
    }

    // ============ 小时级别测试 ============

    #[test]
    fn test_hour1_basic() {
        assert_time_range!(
            "2024/03/21 10:23:45",
            Level::Hour1,
            "2024/03/21 10:00:00",
            "2024/03/21 11:00:00"
        );
    }

    #[test]
    fn test_hour2_various() {
        // 偶数小时
        assert_time_range!(
            "2024/03/21 10:30:00",
            Level::Hour2,
            "2024/03/21 10:00:00",
            "2024/03/21 12:00:00"
        );
        // 奇数小时
        assert_time_range!(
            "2024/03/21 11:59:59",
            Level::Hour2,
            "2024/03/21 10:00:00",
            "2024/03/21 12:00:00"
        );
    }

    #[test]
    fn test_hour4_basic() {
        assert_time_range!(
            "2024/03/21 15:30:00",
            Level::Hour4,
            "2024/03/21 12:00:00",
            "2024/03/21 16:00:00"
        );
    }

    #[test]
    fn test_hour6_cross_day() {
        // 跨天边界: 22:00/04:00
        assert_time_range!(
            "2024/03/21 23:30:00",
            Level::Hour6,
            "2024/03/21 18:00:00",
            "2024/03/22 00:00:00"
        );
        assert_time_range!(
            "2024/03/22 01:15:00",
            Level::Hour6,
            "2024/03/22 00:00:00",
            "2024/03/22 06:00:00"
        );
    }

    #[test]
    fn test_hour12_basic() {
        assert_time_range!(
            "2024/03/21 15:30:00",
            Level::Hour12,
            "2024/03/21 12:00:00",
            "2024/03/22 00:00:00"
        );
        assert_time_range!(
            "2024/03/21 03:30:00",
            Level::Hour12,
            "2024/03/21 00:00:00",
            "2024/03/21 12:00:00"
        );
    }

    // ============ 日级别测试 ============

    #[test]
    fn test_day1_basic() {
        assert_time_range!(
            "2024/03/21 15:30:45",
            Level::Day1,
            "2024/03/21 00:00:00",
            "2024/03/22 00:00:00"
        );
    }

    #[test]
    fn test_day1_cross_month() {
        assert_time_range!(
            "2024/03/31 23:59:59",
            Level::Day1,
            "2024/03/31 00:00:00",
            "2024/04/01 00:00:00"
        );
    }

    #[test]
    fn test_day1_cross_year() {
        assert_time_range!(
            "2024/12/31 12:00:00",
            Level::Day1,
            "2024/12/31 00:00:00",
            "2025/01/01 00:00:00"
        );
    }

    #[test]
    fn test_day3_cross_month() {
        assert_time_range!(
            "2026/02/04 05:00:00",
            Level::Day3,
            "2026/02/03 00:00:00",
            "2026/02/06 00:00:00"
        );
    }

    // ============ 周级别测试 ============

    #[test]
    fn test_week1_monday_start() {
        // 2024/03/18 是周一
        assert_time_range!(
            "2024/03/18 00:00:00",
            Level::Week1,
            "2024/03/18 00:00:00",
            "2024/03/25 00:00:00"
        );
    }

    #[test]
    fn test_week1_midweek() {
        // 2024/03/21 是周四，当周周一是 03/18
        assert_time_range!(
            "2024/03/21 15:30:00",
            Level::Week1,
            "2024/03/18 00:00:00",
            "2024/03/25 00:00:00"
        );
    }

    #[test]
    fn test_week1_cross_month() {
        // 2024/03/31 是周日，当周周一是 03/25
        assert_time_range!(
            "2024/03/31 23:59:59",
            Level::Week1,
            "2024/03/25 00:00:00",
            "2024/04/01 00:00:00"
        );
    }

    #[test]
    fn test_week1_cross_year() {
        // 2023/12/31 是周日，当周周一是 12/25
        assert_time_range!(
            "2023/12/31 12:00:00",
            Level::Week1,
            "2023/12/25 00:00:00",
            "2024/01/01 00:00:00"
        );
    }

    // ============ 月级别测试 ============

    #[test]
    fn test_month1_basic() {
        assert_time_range!(
            "2024/03/21 15:30:00",
            Level::Month1,
            "2024/03/01 00:00:00",
            "2024/04/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_first_day() {
        assert_time_range!(
            "2024/03/01 00:00:00",
            Level::Month1,
            "2024/03/01 00:00:00",
            "2024/04/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_last_day_31() {
        assert_time_range!(
            "2024/01/31 23:59:59",
            Level::Month1,
            "2024/01/01 00:00:00",
            "2024/02/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_last_day_30() {
        assert_time_range!(
            "2024/04/30 12:00:00",
            Level::Month1,
            "2024/04/01 00:00:00",
            "2024/05/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_february_leap_year() {
        // 2024 是闰年
        assert_time_range!(
            "2024/02/29 12:00:00",
            Level::Month1,
            "2024/02/01 00:00:00",
            "2024/03/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_february_common_year() {
        // 2023 不是闰年
        assert_time_range!(
            "2023/02/28 23:59:59",
            Level::Month1,
            "2023/02/01 00:00:00",
            "2023/03/01 00:00:00"
        );
    }

    #[test]
    fn test_month1_cross_year() {
        assert_time_range!(
            "2024/12/15 10:00:00",
            Level::Month1,
            "2024/12/01 00:00:00",
            "2025/01/01 00:00:00"
        );
    }

    // ============ 边界和错误处理测试 ============

    #[test]
    fn test_invalid_timestamp() {
        // 测试无效时间戳（负数或过大）
        let result = get_time_range(i64::MAX as u64, Level::Minute1);
        assert!(result.is_err());
    }

    #[test]
    fn test_epoch_time() {
        // 测试 Unix epoch
        assert_time_range!(
            "1970/01/01 00:00:00",
            Level::Minute1,
            "1970/01/01 00:00:00",
            "1970/01/01 00:01:00"
        );
    }

    #[test]
    fn test_dst_transition() {
        // 注意：由于使用 UTC 时间，不受夏令时影响
        // 这里验证逻辑一致性
        assert_time_range!(
            "2024/03/10 02:30:00", // 美国夏令时开始时间附近
            Level::Hour1,
            "2024/03/10 02:00:00",
            "2024/03/10 03:00:00"
        );
    }
}
