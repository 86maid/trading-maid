# trading-maid

English | [中文](README.zh-CN.md) 

[![Crates.io Version](https://img.shields.io/crates/v/trading-maid?logo=rust)](https://crates.io/crates/trading-maid)
[![docs.rs](https://img.shields.io/docsrs/trading-maid?logo=docs.rs)](https://docs.rs/trading-maid)
[![GitHub Repo stars](https://img.shields.io/github/stars/86maid/trading-maid)](https://github.com/86maid/trading-maid)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-2563eb.svg)](https://opensource.org/licenses/Apache-2.0)

> ⚡ Keywords: high-fidelity matching / two-stage trigger orders / margin and liquidation mechanics / backtest visualization

trading-maid is a backtesting and live-trading framework for crypto futures, with a strong focus on behavior close to real exchanges.
It includes key mechanics such as matching, slippage, leverage, margin, and liquidation for strategy validation, iteration, and live integration.

![trading-maid](a.gif)

## Contents

- [✨ Core Capabilities](#-core-capabilities)
- [🧭 Trading Model and Constraints](#-trading-model-and-constraints)
- [🏗️ Architecture Overview](#-architecture-overview)
- [🚀 Quick Start](#-quick-start)
- [⚡ Fast Backtest](#-fast-backtest)
- [🧠 Context](#-context)
- [📊 Series](#-series)
- [📈 Indicators](#-indicators)
- [🧩 Strategy as Struct](#-strategy-as-struct)
- [🛑 Error Handling](#-error-handling)
- [🚦 Hook Intercept](#-hook-intercept)
- [📊 Custom Series](#-custom-series)
- [🔄 Getting Data at Other Levels](#-getting-data-at-other-levels)
- [🌐 Multi-Asset Strategy](#-multi-asset-strategy)
- [💰 Exchange Account Methods](#-exchange-account-methods)
- [🧰 Utility Functions](#-utility-functions)
- [🧪 EMA Strategy Example](#-ema-strategy-example)
- [📊 Shadow Reversal Example](#-shadow-reversal--candle-wick-reversal-built-in-indicators)
- [📊 Volume Breakout Example](#-volume-breakout--donchian--volume-spike-custom-indicators)
- [📊 Price Action Example](#-price-action--momentum-breakout-custom-indicators)
- [📊 RSI EMA Example](#-rsi-ema--consecutive-candle-breakout-built-in-indicators)
- [📊 Vegas Martingale Example](#-vegas-martingale--vegas-tunnel--rsimacd-built-in-indicators)
- [📊 Horizon Long Example](#-horizon-long--time-series-momentum-custom-indicators)

## ✨ Core Capabilities

- **Backtesting environment close to live trading**: simulates exchange matching logic, with built-in **slippage**, **leverage**, **margin**, and **forced liquidation** mechanics to reduce backtest/live deviation.
- **Live exchange abstraction**: provides a unified exchange interface for smooth migration from backtest to live trading.
- **Indicator and series tools**: includes common technical indicators and time-series processing utilities.
- **Backtest result visualization**: render candlesticks, orders, and position history in a web page.

## 🧭 Trading Model and Constraints

### 🧾 Order Types

- Supported: trigger price + (limit | market)
- Not supported: OCO (take-profit/stop-loss combo order; Binance does not support it either)

### 📦 Position Type

- Margin mode: isolated
- Position direction: one-way
- Margin asset type: single-currency margin
- Margin management: dynamically adjusts position margin

### ⚙️ Order Processing Logic

- **Market order**: fills at `Open` (slippage applied, then clamped to `[Low, High]`).
- **Limit order**:
    - Buy: fills once `Low <= price`, at `min(Open, price)`
    - Sell: fills once `High >= price`, at `max(Open, price)`
    - In other words: if the open is already better than your price, you get the open (price improvement); otherwise you get your price. No slippage.
- **Trigger order**: once the trigger price is touched, it turns into a market or limit order **on the same k-line** and fills by the rules above.
    - The trigger direction is fixed at placement: trigger above the current close → fires when `High >= trigger`; below → fires when `Low <= trigger`.
    - Direction is independent of side: `sell_trigger_market(110.5)` with the market at 105 means "sell once the price rises to 110.5".
    - When it turns into a market order the fill is `max/min(Open, trigger)`: gapped past the trigger → fill at the open; crossed intrabar → fill at the trigger.
- **Timing**: an order placed on a k-line is not matched on that k-line; matching starts on the next one. Only trigger orders can complete "trigger → fill" within the same k-line.
- **Margin freeze**: market orders freeze at fill; limit orders freeze at placement based on their price (any excess is refunded when the fill is better); trigger orders freeze when triggered; reduce-only orders never freeze.
- **Priority**: within a k-line, orders are matched in placement order; pending orders run before liquidation.
- **Fees**: market orders and trigger-to-market use `taker_fee`; limit orders and trigger-to-limit use `maker_fee`; liquidation uses `taker_fee`.

## 🏗️ Architecture Overview

For the full text diagram, see [architecture.txt](architecture.txt).

## 🚀 Quick Start

### 📥 Installation

Using `cargo add`

```bash
cargo add trading-maid
```

Or in `Cargo.toml`

```toml
[dependencies]
trading-maid = "1"
```

### 🛠️ Usage

```rust
use trading_maid::prelude::*;

// Open a short position when a long upper shadow appears.
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let body_size = (cx.open - cx.close).abs();
    let upper_shadow_size = (cx.high - cx.open).abs();
    let open_short_condition =
        cx.open > cx.close && upper_shadow_size >= body_size * 2 && body_size >= 300;

    if cx.get_position("BTCUSDT").await?.is_none() && open_short_condition {
        println!("place order: {}", t2s(cx.time));

        let take_profit_price = cx.open - upper_shadow_size;
        let stop_price = cx.open + upper_shadow_size;

        cx.cancel_all_order("BTCUSDT").await?;

        _ = cx
            .sell_tp_sl("BTCUSDT", take_profit_price, stop_price, "0.01")
            .await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    // Download the latest 12 months of 1-minute-level data.
    let path = get_or_download("BTCUSDT/1m", 12).await.unwrap();

    let data_source_1m = DataSource::from_file_metadata(
        path,
        Metadata {
            symbol: "BTCUSDT".to_string(),
            level: Level::Minute1,
            min_size: "0.01".parse().unwrap(),
            min_notional: "0".parse().unwrap(),
            tick_size: "0.1".parse().unwrap(),
            maker_fee: "0.0002".parse().unwrap(),
            taker_fee: "0.0005".parse().unwrap(),
            maintenance: "0.004".parse().unwrap(),
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .unwrap()
        .cash(10000)
        .leverage(10)
        .slippage(0);

    let mut engine = Engine::new(exchange.clone(), my_strategy);

    // Backtest with 1-minute data, but call the strategy whenever each 1-hour k-line is generated.
    if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
        println!("error: {:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    // Resample 1-minute data into 1-hour data.
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    // Pass multiple time levels for easier switching in the visualization.
    open_in_server(
        [data_source_1h, data_source_1m],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
```

In this example, we set:

* minimum order size (min_size): 0.01
* minimum notional (min_notional): 0.0 = no restriction
* price tick size (tick_size): 0.1
* maker fee (maker_fee): 0.0002
* taker fee (taker_fee): 0.0005
* maintenance margin rate (maintenance): 0.004
* cash: 10000
* leverage: 10
* slippage: 0

**The backtest runs on 1-minute data while the strategy runs on 1-hour k-lines.** The engine calls the strategy at each 1-hour k-line close (the last minute of each hour), and every k-line observed by the strategy is 1-hour-level.

Although other levels can be used, you should **always use 1-minute data as the backtest source** to achieve high-precision results.

`open_in_server` starts a local server and automatically opens the backtest visualization page in the browser. 

Prefer `open_in_server` over `open_in_browser` — the latter writes k-line data to a file each time, causing the browser to reload and wasting time.

Use `cargo run -r` to run backtests faster.

> ⚠️ Note: `sell_tp_sl` is syntactic sugar, not a real OCO order (OCO is not supported by the framework). It simply places two orders at once — you still need to call `cancel_all_order` to cancel old orders before opening a new position.

> ⚠️ **Stop Loss**: Stop loss should use trigger orders. For example, after a market buy, use `sell_trigger_market_reduce_only` (trigger price reached → reduce-only market sell); after a market sell, use `buy_trigger_market_reduce_only` (trigger price reached → reduce-only market buy). Do NOT use `sell_limit_reduce_only` or `buy_limit_reduce_only` — in an order book, a sell limit below market (or buy limit above market) crosses the spread and matches instantly, turning your stop loss into an immediate market exit instead of waiting for the price to reach your stop. Simply put, your limit order fills instantly at the market price. Also, always use `reduce_only` — without it, the order may open a reverse position instead of closing your current one.

> ⚠️ **Precision Warning**: When creating orders (e.g., `buy`, `sell`, `buy_limit`, `sell_tp_sl`, etc.), price and quantity parameters accept `impl TryInto<Decimal>`. To avoid floating-point precision loss, pass high-precision values as strings (e.g., `"0.01"`) instead of `f64` literals like `0.01`.

## ⚡ Fast Backtest

For quick backtesting with sensible defaults, use the `backtest()` function — it handles data downloading, exchange setup, and engine creation automatically. One function call is all you need:

```rust
use trading_maid::prelude::*;

// Open a short position when a long upper shadow appears.
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let body_size = (cx.open - cx.close).abs();
    let upper_shadow_size = (cx.high - cx.open).abs();
    let open_short_condition =
        cx.open > cx.close && upper_shadow_size >= body_size * 2 && body_size >= 300;

    if cx.get_position("BTCUSDT").await?.is_none() && open_short_condition {
        println!("place order: {}", t2s(cx.time));

        let take_profit_price = cx.open - upper_shadow_size;
        let stop_price = cx.open + upper_shadow_size;

        cx.cancel_all_order("BTCUSDT").await?;

        _ = cx
            .sell_tp_sl("BTCUSDT", take_profit_price, stop_price, "0.01")
            .await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour1)
        .await
        .unwrap();

    // Backtest result summary
    println!("summary: {:#?}", result.summarize());

    // Visualization with data resampled to all compatible levels
    result.resample_all_open_in_server().await.unwrap();
}
```

`backtest()` uses the following preset configurations:

* **Data source**: 1-minute level (auto-downloaded via `get_or_download`)
* **Metadata**: min_size=0.01, min_notional=0, tick_size=0.1, maker_fee=0.0002, taker_fee=0.0005, maintenance=0.004
* **Exchange**: cash=1,000,000, leverage=1, slippage=0

### BacktestResult API

The returned `BacktestResult` provides:

| Method | Description |
|--------|-------------|
| `summarize()` | Returns a `HistoryPositionSummary` with key metrics (win rate, profit/loss, total trades, etc.) |
| `open_in_browser()` | Writes visualization to a temp HTML file and opens it in the default browser |
| `open_in_server()` | Starts a local server for visualization at the strategy level |
| `resample_all_open_in_server()` | Starts a local server with data resampled to all compatible levels for easy switching |

> 💡 **This is the lazy approach** — perfect for rapid strategy prototyping and quick experiments. For full control over fees, leverage, slippage, cash, and other parameters, use the manual setup shown in [Quick Start](#-quick-start).

## 🧠 Context

In `Context`, `time`, `open`, `high`, `low`, and `close` are of type `&Series`, which is essentially a wrapped slice.

You can use `cx.close[0]` for the current k-line close, `cx.close[1]` for the previous k-line close, and so on.

You can use `&cx.close[2..]` to get a slice.

This type also overloads many operators, so you can omit index `[0]` in calculations, for example `cx.close + 100`.

You can destructure `Context` for easier OHLCV usage.

```rust
async fn my_strategy(
    Context {
        time,
        open,
        high,
        low,
        close,
        volume,
        exchange,
        series,
    }: &Context<'_>,
) -> anyhow::Result<()> {
    println!("time: {}", t2s(time));
    Ok(())
}
```

### 🕳️ Gap Detection: `cx.missed`

K-lines can be lost — a websocket reconnect, a stalled process, or an archive that simply never published a bar. The engine skips the hole and reports it through `cx.missed`, which is `None` on a normal bar and otherwise tells you **which kind** of hole it was:

```rust
pub enum Missed {
    /// The bars exist but the strategy was not called: `close[1]`, `close[2]`, ...,
    /// `close[n]` are them, in order. The series stays continuous.
    Backfilled(usize),
    /// The bars do not exist: `time` jumps, and `close[1..=n]` cannot be read.
    Missing(usize),
}
```

In both variants `n` means the same thing: this call covers `n + 1` strategy periods instead of one.

**`Backfilled`** — a live exchange dropped bars and the engine recovered them through `get_kline`. The values are in the series, so a stateful strategy can replay them and stay exact:

```rust
match cx.missed {
    Some(Missed::Backfilled(n)) => {
        for i in (1..=n).rev() {
            self.ema_cache.update(cx.close[i]);
        }
    }
    _ => {}
}
```

**`Missing`** — the bars are not in the data at all (a hole in a historical archive). There is nothing to replay: `close[1]` belongs to the bar before the hole, and an out-of-range index returns the `Decimal::MAX` sentinel, which would poison any indicator. The only correct response is to reset the incremental state:

```rust
if let Some(Missed::Missing(_)) = cx.missed {
    self.ema_cache = EMACache::new(self.length);
}
```

A strategy that ignores `cx.missed` is only correct while it is `None` — either handle both variants, or rebuild from the full history with `EMACache::from_series`.

Two more things worth knowing:

- Any strategy-level bar whose source bars were affected by a hole is **dropped whole**, never delivered with a partial volume/high/low that still looks perfectly contiguous. That is why `Missing` always agrees with a jump in `cx.time`.
- `cx.request(...)` exposes the same field for the symbol it returns.

Real archives do have such holes. **Binance's futures archive is missing 2026-10-28 entirely**:
the monthly `BTCUSDT-1m-2026-10.zip` holds 43200 rows instead of the 44640 a 31-day month should
have, the 1h file is short by 24 bars, and the daily `2026-10-28.zip` returns 404 — while the
**spot archive for that day is complete**. Nothing is ever filled in: a 12/24/36/48-month futures
backtest simply skips that day, the strategy-level bars it covers are dropped whole, and the next
call reports `Missed::Missing(n)`.

## 📊 Series

`Series` is the core numeric type used throughout `Context`. It wraps a reversed slice — `series[0]` is the current bar, `series[1]` the previous, and so on. Out-of-bounds indices return `Decimal::MAX`.

`Series` overloads a wide range of operators, so you can write natural expressions:

```rust
// Arithmetic — omits [0] automatically
cx.close + 100;
cx.high - cx.low;
cx.close * dec!(1.5);

// Comparison — compare directly with numbers or strings
cx.close == 123;
cx.close > 123.456;
cx.close < "0.0005";

// Slicing — returns &Series
&cx.close[2..];     // from 2 bars ago to the start
&cx.close[..5];     // most recent 5 bars
&cx.close[2..5];    // range of bars
```

> ⚠️ **Precision Warning:** Avoid `f64` literals — use strings (`"123.456"`) or the `dec!` macro instead to prevent floating-point precision loss.

## 📈 Indicators

Some commonly used indicators are built into `indicator`.

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    highest(cx.high, 7);
    sma(cx.close, 30);
    ema(cx.close, 144);
    Ok(())
}
```

Calling `ema` directly may lead to slow backtests and incorrect calculations, because EMA depends on the previous EMA value. So it is recommended to use `EMACache` for fast and accurate calculation.

Use `EMACache::with_ema` to create an instance with an initial EMA value, then call `EMACache::update` on each k-line to update and get the current value:

When the data does not start from the very first k-line (live trading, a preloaded window, or a narrower backtest range), rebuild the cache from history with `EMACache::from_series(cx.close, 144)` instead of hard-coding a number.

```rust
async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
    let Some(ema144) = self.ema_cache144.update(cx.close) else {
        return Ok(());
    };
    
    Ok(())
}
```

## 🧩 Strategy as Struct

When your strategy needs to maintain state (e.g., caches, counters), implement the `Strategy` trait on a struct.

```rust
struct MyStrategy {
    ema_cache: EMACache,
    count: usize,
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        // use self.ema_cache, self.count ...
        Ok(())
    }
}

let mut engine = Engine::new(exchange.clone(), MyStrategy::new());
```

## 🛑 Error Handling

Use `on_error` to handle errors returned by the strategy. If not set, the engine stops immediately when an error occurs.

```rust
let mut engine = Engine::new(exchange.clone(), MyStrategy::new());

engine.on_error(|err| {
    eprintln!("strategy error: {:#?}", err);
    Ok(()) // return Ok to continue, or Err to stop the engine
});

if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
    println!("{:#?}", v);
}
```

> 💡 Unlike `hook` which runs on every k-line, `on_error` is only called when the strategy returns an error.

## 🚦 Hook Intercept

You can use `hook` to stop backtesting when a position is liquidated or an order is rejected (insufficient balance).

The hook function is called after the strategy executes on each k-line. Its second argument carries the k-lines that had to be backfilled after a network interruption (empty on a normal bar), so the hook's own state can catch up.

```rust
async fn my_hook(_: KLine, _: &[KLine], exchange: ExchangeWrapper) -> anyhow::Result<()> {
    if let Some(v) = exchange
        .get_history_order_list("BTCUSDT")
        .await?
        .iter()
        .find(|v| v.status == Status::Rejected || v.kind == Kind::Liquidation)
    {
        anyhow::bail!(
            "rejected/liquidation {}: cash: {}",
            t2s(v.update_time),
            exchange.get_cash().await?
        );
    }

    Ok(())
}

let mut engine = Engine::new(exchange.clone(), MyStrategy::new());

engine.hook(my_hook);

if let Err(v) = engine.run("BTCUSDT", Level::Minute5).await {
    println!("{:#?}", v);
}
```

## 📊 Custom Series

Use `add_series` to attach custom data (funding rate, on-chain metrics, sentiment, etc.) to the engine. Once registered, the series is synchronised with the OHLCV data and accessible in the strategy via `cx["name"]`.

### Aligning Custom Data

Custom data usually comes as sparse `(timestamp_ms, value)` pairs. Use `align_to_series` to forward-fill these into an `AlignedSeries` at a target k-line level:

```rust
// Sparse custom data: (timestamp_ms, value)
let custom_data = vec![
    (1717200000000, "1.5".parse::<Decimal>().unwrap()),
    (1717286400000, "2.3".parse::<Decimal>().unwrap()),
];

// Forward-fill align to 1-hour bars
let series = align_to_series(&custom_data, Level::Hour1).unwrap();

// Register with the engine before calling run()
let mut engine = Engine::new(exchange, my_strategy);
engine.add_series("BTCUSDT", "custom_metric", series);
engine.run("BTCUSDT", Level::Hour1).await?;
```

### Funding Rate Example

`get_or_download_funding_rate_to_series` downloads Binance funding rate history and aligns it automatically:

```rust
let funding_rate_series = get_or_download_funding_rate_to_series(
    "BTCUSDT", 12, Level::Hour1,
).await.unwrap();

engine.add_series("BTCUSDT", "funding_rate", funding_rate_series);
engine.run("BTCUSDT", Level::Hour1).await?;
```

### Access in Strategy

Read the additional series by name inside the strategy:

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if cx["funding_rate"] != &[] {
        // Current bar's funding rate
        let fr = cx["funding_rate"][0];
        // Previous bar
        let fr_prev = cx["funding_rate"][1];

        // Avoid longing when funding is too high
        if fr > "0.0005" {
            return Ok(());
        }
    }

    Ok(())
}
```

If no series is registered for the given symbol/level/name, `cx[name]` returns an empty series (compare with `== []`).

## 🔄 Getting Data at Other Levels

Use `cx.request()` to obtain a symbol's OHLCV context at a **specific level**.

```rust
// Get BTCUSDT at 5-minute level
if let Some(btc_5m_cx) = cx.request("BTCUSDT", Level::Minute5) {
    let sma_30 = sma(btc_5m_cx.close, 30);
}
```

`request` is powered by **resampling**:

- If the requested level matches the **strategy level** (the level passed to `engine.run`) or the **source data level** (the DataSource's level), pre-built data is returned instantly with zero overhead.
- For any other level, the framework resamples from the source kline data to the target level. The result is **computed on first access and cached** — subsequent calls within the same bar hit the cache and incur no extra cost.

The target interval must be an integer multiple of the source data interval. Use `is_valid_sampling_target` to check compatibility.
Monthly bars are the exception: their length follows the calendar (28–31 days), so they are only generated from source levels whose interval divides one day (1m–12h, 1d). Aggregating 3d or 1w bars into a month would assign a bar that straddles the month boundary entirely to the earlier month, so it is rejected.

> ⚠️ **Recommendation: always use 1‑minute data as the source.** This allows resampling to any coarser level for maximum flexibility.

## 🌐 Multi-Asset Strategy

Run a strategy across multiple symbols simultaneously. Pass an array of symbols to `run()` and use `cx.request()` to access other symbols' OHLCV data:

```rust
let exchange = LocalExchange::new([btc_data, eth_data])?;

let mut engine = Engine::new(exchange, my_strategy);

// Pass multiple symbols to run()
engine.run(["BTCUSDT", "ETHUSDT"], Level::Hour1).await?;
```

In the strategy, access another symbol's context via `cx.request()`:

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // Access BTCUSDT directly (primary symbol)
    let sma_val = sma(cx.close, 30);

    // Access ETHUSDT via request
    if let Some(eth_cx) = cx.request("ETHUSDT", Level::Hour1) {
        // Use eth_cx like a normal Context
        let sma_val = sma(eth_cx.close, 30);
    }
    
    Ok(())
}
```

> ⚠️ **Note:** All symbols must share the same level, and their data time ranges must overlap — the strategy only fires when all symbols have data at the current bar.
>
> ⚠️ **Note:** The engine calls `next` concurrently for each symbol.

## 💰 Exchange Account Methods

### Querying Account State

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // Total equity = cash + unrealized PnL of all positions.
    let equity = cx.get_equity().await?;

    // Available cash balance.
    let cash = cx.get_cash().await?;

    // Leverage currently configured for the symbol.
    let leverage = cx.get_leverage("BTCUSDT").await?;

    println!("equity: {}, cash: {}, leverage: {}x", equity, cash, leverage);

    Ok(())
}
```

> 💡 `get_equity` counts **unrealized** PnL, so it moves with price on every bar, while `get_cash` only reflects realized changes. `equity - cash` is therefore a quick read on open position performance.

### Adjusting Leverage

`set_leverage` changes the leverage **and immediately re-computes the position margin** from the open average price and quantity. Raising leverage frees margin back to cash; lowering it requires extra cash.

```rust
// Set the leverage for the symbol to 20x.
cx.set_leverage("BTCUSDT", 20).await?;
```

Because margin is recalculated, `set_leverage` can fail. Handle those errors rather than unwrapping:

- **Pending orders block the call**: if the symbol has any open order, the leverage cannot be changed. Cancel them first with `cx.cancel_all_order(symbol).await?`.
- **Insufficient cash**: lowering leverage increases the required margin; if `cash` cannot cover the shortfall, the call fails and the position is left untouched.

### Adjusting Position Margin to Avoid Liquidation

`append_position_margin` adds margin to an existing position. Extra margin pushes the **liquidation price further away** from the current price, which is the standard way to rescue a position that is drifting toward forced liquidation.

Where the liquidation price sits before opening a position can be predicted with `calc_liquidation_price`, which takes the leverage, the maintenance margin rate from the symbol metadata, the position side, entry price, quantity, and total position margin:

```rust
let metadata = cx.get_metadata("BTCUSDT").await?;

let liquidation_price = calc_liquidation_price(
    20,                            // leverage
    metadata.maintenance,          // maintenance margin rate
    Side::Buy,
    "60000".parse().unwrap(),      // entry price
    "0.5".parse().unwrap(),        // quantity
    "1500".parse().unwrap(),       // total position margin
);
```

So for a long, adding margin **lowers** the liquidation price (safer); for a short, adding margin **raises** it. Passing a negative `margin` withdraws margin and tightens the liquidation price.

```rust
// Add 500 to the position margin, moving the liquidation price further away.
cx.append_position_margin("BTCUSDT", "500").await?;

// Withdraw 500 back to cash, moving the liquidation price closer.
cx.append_position_margin("BTCUSDT", "-500").await?;
```

Guard rails to be aware of:

- **Margin cannot be reduced below the initial margin** of the position (`entry_price * quantity / leverage`); attempting it fails with the minimum required margin in the message.

### Detecting Liquidation

In the position history, `is_liquidation()` tells whether a position was force-closed:

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if let Some(last) = cx.get_history_position_list("BTCUSDT").await?.last()
        && last.is_liquidation()
    {
        anyhow::bail!("position was liquidated at {}", t2s(last.close_time));
    }

    Ok(())
}
```

### 🛫 Preloading History (Live Only)

A freshly started live strategy is blind until its indicators have seen enough bars — `sma(cx.close, 200)` returns `None` for the first 200 bars. Preloading pulls history before the first live k-line so the strategy is usable immediately:

```rust
// Take the last 200 bars before the first live k-line.
engine.preload(200);

// ...or everything from this timestamp onward (0 = as early as the exchange allows).
engine.preload_from(1735689600000);

// ...or seed from data you already hold in memory.
engine.preload_data(data_source);
```

Only one setting applies; the last call wins. Preloading is ignored in backtests, where the data source already starts from its first bar.

The preloaded range ends **exactly one bar before** the first live k-line, so `cx` stays contiguous and never duplicates that bar. If the gap between the seed and the first live bar cannot be filled through `get_kline`, the engine aborts rather than starting with a hole.

Preloading fills `cx`, not your own state: an `EMACache` still starts from zero unless you seed it with `EMACache::from_series(cx.close, 144)` on the first call.

## 🧰 Utility Functions

```rust
use trading_maid::prelude::*;

// Convert a Unix timestamp in milliseconds to a local-time string.
t2s(1717200000000u64);

// Convert a Unix timestamp in milliseconds to a UTC string.
t2s_utc(1717200000000u64);

// Convert a local-time string back to a Unix timestamp in milliseconds.
s2t("2024/06/01 04:00:00");

// Convert a UTC string back to a Unix timestamp in milliseconds.
s2t_utc("2024/06/01 04:00:00");

// Get the start of the current bar and the start of the next bar.
get_time_range(1717200000000, Level::Hour1)?;

// Get the end time of the coarser bar that contains the given time.
get_last_time(1717200000000, Level::Minute1, Level::Hour1)?;

// Round a price to the nearest valid tick size.
round_to_tick(dec!(60000.37), dec!(0.1));

// Check whether a price is aligned to the tick size.
is_tick_aligned("60000.4".parse().unwrap(), "0.1".parse().unwrap());

// Calculate the initial margin: price * quantity / leverage.
calc_initial_margin("60000".parse().unwrap(), "0.5".parse().unwrap(), 20);

// Calculate the liquidation price for a position.
calc_liquidation_price(20, "0.004".parse().unwrap(), Side::Buy, "60000".parse().unwrap(), "0.5".parse().unwrap(), "1500".parse().unwrap());

// Forward-fill sparse (timestamp, value) pairs onto a k-line level.
align_to_series(&[(1717200000000u64, "1.5".parse().unwrap())], Level::Hour1)?;

// Resample a list of k-lines to a coarser level.
resample(&kline_list, Level::Hour1)?;

// Resample a data file into every coarser level, writing each next to it.
resample_file("BTCUSDT-1m.csv")?;

// Download k-line data, or reuse the local cache if present.
get_or_download("BTCUSDT/1m", 12).await?;

// Download funding rate history.
get_or_download_funding_rate("BTCUSDT", 12).await?;

// Download funding rate history and align it to a k-line level.
get_or_download_funding_rate_to_series("BTCUSDT", 12, Level::Hour8).await?;

// Render data sources, positions and orders into an HTML string.
to_html(&data_source_list, &history_position_list, &history_order_list);

// Render into a temp HTML file and open it in the browser.
open_in_browser(&data_source_list, &history_position_list, &history_order_list)?;

// Serve the rendered result from a local web server and open it.
open_in_server(data_source_list, history_position_list, history_order_list).await?;

// Aggregate a list of closed positions into summary statistics.
summarize(&history_position_list);
```

## 📐 Strategy Examples

### 🧪 demo_indicator — EMA example

The smallest runnable example, showing how to use a built-in indicator (`EMACache`): go short when the last 50 bars closed below the EMA and the current bar closes above it.
The two hard-coded EMA seed values only exist so the example can produce an indicator from its very first bar; they go stale as soon as the data range changes — use `EMACache::from_series` in a real strategy.

```rust
use trading_maid::prelude::*;

struct MyStrategy {
    ema_cache144: EMACache,
    ema_cache169: EMACache,
    count: usize,
}

impl MyStrategy {
    pub fn new() -> Self {
        // 80871 / 78705 是从某个日期那根 5m K 线算出来的 EMA 值，写死只是为了让示例从第一根
        // bar 就能算出指标。换数据区间、换起始日期，这两个数就失效了；实盘或换区间时应该改用
        // EMACache::from_series(cx.close, 144) 从历史重建。
        MyStrategy {
            ema_cache144: EMACache::with_ema(144, 80871),
            ema_cache169: EMACache::with_ema(169, 78705),
            count: 0,
        }
    }
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    // 如果连续 50 根 K 线收盘价都在 EMA 之下，且当前 K 线收盘价突破 EMA，则开空单
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let Some(ema144) = self.ema_cache144.update(cx.close) else {
            return Ok(());
        };

        let Some(ema169) = self.ema_cache169.update(cx.close) else {
            return Ok(());
        };

        if self.count >= 50
            && (cx.close >= ema144 || cx.close >= ema169)
            && cx.get_position("BTCUSDT").await?.is_none()
        {
            println!("place_order: {}", t2s(cx.time));

            cx.cancel_all_order("BTCUSDT").await?;

            _ = cx
                .sell_tp_sl("BTCUSDT", cx.close - 1000, cx.close + 1000, "0.01")
                .await?;
        }

        if cx.close <= ema144 && cx.close <= ema169 {
            self.count += 1;
        } else {
            self.count = 0;
        }

        Ok(())
    }
}

async fn my_hook(_: KLine, _: &[KLine], exchange: ExchangeWrapper) -> anyhow::Result<()> {
    if let Some(v) = exchange
        .get_history_order_list("BTCUSDT")
        .await?
        .iter()
        .find(|v| v.status == Status::Rejected || v.kind == Kind::Liquidation)
    {
        anyhow::bail!(
            "rejected/liquidation {}: cash: {}",
            t2s(v.update_time),
            exchange.get_cash().await?
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let path = get_or_download("BTCUSDT/1m", 12).await.unwrap();

    let data_source_1m = DataSource::from_file_metadata(
        path,
        Metadata {
            symbol: "BTCUSDT".to_string(),
            level: Level::Minute1,
            min_size: "0.01".parse().unwrap(),
            min_notional: "0".parse().unwrap(),
            tick_size: "0.1".parse().unwrap(),
            maker_fee: "0.0002".parse().unwrap(),
            taker_fee: "0.0005".parse().unwrap(),
            maintenance: "0.004".parse().unwrap(),
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .unwrap()
        .cash(10000)
        .leverage(10)
        .slippage(0);

    let mut engine = Engine::new(exchange.clone(), MyStrategy::new());

    engine.hook(my_hook);

    if let Err(v) = engine.run("BTCUSDT", Level::Minute5).await {
        println!("error: {:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    let data_source_5m = data_source_1m.resample(Level::Minute5).unwrap();
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    open_in_server(
        [data_source_5m, data_source_1m, data_source_1h],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
```

### 📦 demo_minimal

Go short on a long upper wick and attach take-profit/stop — the least code that runs end to end.

```rust
use trading_maid::prelude::*;

// 出现长上影线时开空单
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let body_size = (cx.open - cx.close).abs();
    let upper_shadow_size = (cx.high - cx.open).abs();
    let open_short_condition =
        cx.open > cx.close && upper_shadow_size >= body_size * 2 && body_size >= 300;

    if cx.get_position("BTCUSDT").await?.is_none() && open_short_condition {
        println!("place order: {}", t2s(cx.time));

        let take_profit_price = cx.open - upper_shadow_size;
        let stop_price = cx.open + upper_shadow_size;

        cx.cancel_all_order("BTCUSDT").await?;

        _ = cx
            .sell_tp_sl("BTCUSDT", take_profit_price, stop_price, "0.01")
            .await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let path = get_or_download("BTCUSDT/1m", 12).await.unwrap();

    let data_source_1m = DataSource::from_file_metadata(
        path,
        Metadata {
            symbol: "BTCUSDT".to_string(),
            level: Level::Minute1,
            min_size: "0.01".parse().unwrap(),
            min_notional: "0".parse().unwrap(),
            tick_size: "0.1".parse().unwrap(),
            maker_fee: "0.0002".parse().unwrap(),
            taker_fee: "0.0005".parse().unwrap(),
            maintenance: "0.004".parse().unwrap(),
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .unwrap()
        .cash(10000)
        .leverage(10)
        .slippage(0);

    let mut engine = Engine::new(exchange.clone(), my_strategy);

    // 使用 1 分钟级别数据进行回测，但在每个 1 小时级别的 K 线生成时都会调用策略函数
    if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
        println!("error: {:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    // 从 1 分钟级别数据重采样得到 1 小时级别数据
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    open_in_server(
        [data_source_1h, data_source_1m],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
```

### 💰 demo_funding_rate

Download the funding rate as a custom series and go long when five consecutive periods are negative — shows `add_series` and `cx["funding_rate"]`.

```rust
use trading_maid::prelude::*;

// 资金费率为负的时候做多
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let funding_rate = &cx["funding_rate"];

    if funding_rate != &[] {
        if cx.get_position("BTCUSDT").await?.is_none()
            && funding_rate < 0
            && funding_rate[1] < 0
            && funding_rate[2] < 0
            && funding_rate[3] < 0
            && funding_rate[4] < 0
        {
            println!(
                "place order: {}, funding_rate: {}",
                t2s(cx.time),
                funding_rate
            );

            let take_profit_price = cx.open + 1000;
            let stop_price = cx.open - 1000;

            cx.cancel_all_order("BTCUSDT").await?;

            _ = cx
                .buy_tp_sl("BTCUSDT", take_profit_price, stop_price, "0.01")
                .await?;
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let path = get_or_download("BTCUSDT/1m", 12).await.unwrap();

    let data_source_1m = DataSource::from_file_metadata(
        path,
        Metadata {
            symbol: "BTCUSDT".to_string(),
            level: Level::Minute1,
            min_size: "0.01".parse().unwrap(),
            min_notional: "0".parse().unwrap(),
            tick_size: "0.1".parse().unwrap(),
            maker_fee: "0.0002".parse().unwrap(),
            taker_fee: "0.0005".parse().unwrap(),
            maintenance: "0.004".parse().unwrap(),
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .unwrap()
        .cash(10000)
        .leverage(10)
        .slippage(0);

    let mut engine = Engine::new(exchange.clone(), my_strategy);

    engine.add_series(
        "BTCUSDT",
        "funding_rate",
        get_or_download_funding_rate_to_series("BTCUSDT", 12, Level::Hour8)
            .await
            .unwrap(),
    );

    if let Err(v) = engine.run("BTCUSDT", Level::Hour8).await {
        println!("error: {:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    let data_source_4h = data_source_1m.resample(Level::Hour4).unwrap();
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    open_in_server(
        [data_source_4h, data_source_1h, data_source_1m],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
```

## 🏆 Strategy Leaderboard

Every `strategy_*` file under `examples/` is a complete strategy — entry/exit rules, position sizing, a full backtest. The complete source lives in the file of the same name; the leaderboard below is what that code produces.

Same setup for every row: BTCUSDT futures, 1m source data, starting cash 1,000,000, position size set by each strategy (mostly 0.01 BTC). The absolute numbers are therefore tiny — **read the ordering, profit/loss ratio and drawdown, not a percentage return**. The 2026-10-28 archive hole is skipped.

Max drawdown is sampled from the account equity on every source-level k-line by `DrawdownHook`, so it **includes floating losses**; summing closed positions understates it.

Every window runs **the same code with parameters untouched**. 3 and 6 months are far too small to mean anything; judge robustness from 12 months upwards.

### 3 months (2026-08 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 15 | 20.00% | 7.15 | 27.05 | **405.78** | 456.97 |
| 2 | `strategy_volume_breakout` | 8 | 62.50% | 1.42 | 13.56 | 108.46 | 102.90 |
| 3 | `strategy_rsi_ema` | 16 | 43.75% | 1.92 | 4.64 | 74.27 | 176.67 |
| 4 | `strategy_fast` | 4 | 50.00% | 2.31 | 3.38 | 13.52 | 13.60 |
| 5 | `strategy_pullback_continuation` | 5 | 40.00% | 1.94 | 2.37 | 11.84 | 31.59 |
| 6 | `strategy_vegas_martingale` | 0 | 0.00% | 0.00 | 0.00 | 0.00 | 0.00 |
| 7 | `strategy_price_action` | 8 | 25.00% | 2.28 | -3.19 | -25.51 | 118.23 |
| 8 | `strategy_shadow_reversal` | 8 | 12.50% | 1.52 | -11.22 | -89.76 | 109.78 |
| 9 | `strategy_squeeze_dual_structure` | 14 | 14.29% | 2.46 | -7.75 | -108.57 | 139.50 |
| 10 | `strategy_dual_structure_trend` | 14 | 14.29% | 2.46 | -7.75 | -108.57 | 139.50 |
| 11 | `strategy_pullback_plus_breakout` | 14 | 14.29% | 2.46 | -7.75 | -108.57 | 139.50 |
| 12 | `strategy_pullback_trend` | 11 | 9.09% | 2.36 | -10.45 | -114.93 | 125.69 |
| 13 | `strategy_fib_martingale` | 10 | 30.00% | 0.67 | -11.74 | -117.38 | 141.75 |
| 14 | `strategy_fib_martingale_050` | 14 | 42.86% | 0.42 | -9.73 | -136.26 | 171.23 |
| 15 | `strategy_fib_martingale_1h_050` | 63 | 44.44% | 0.81 | -2.24 | -141.22 | 160.13 |
| 16 | `strategy_fib_martingale_1h` | 61 | 31.15% | 1.23 | -2.87 | -175.26 | 179.91 |

### 6 months (2026-05 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 30 | 30.00% | 3.18 | 10.31 | **309.43** | 456.97 |
| 2 | `strategy_volume_breakout` | 26 | 42.31% | 1.72 | 3.22 | 83.66 | 224.29 |
| 3 | `strategy_rsi_ema` | 37 | 35.14% | 2.01 | 0.93 | 34.27 | 212.93 |
| 4 | `strategy_fast` | 6 | 33.33% | 1.58 | -1.07 | -6.41 | 48.50 |
| 5 | `strategy_pullback_continuation` | 13 | 46.15% | 1.07 | -0.64 | -8.38 | 77.59 |
| 6 | `strategy_pullback_plus_breakout` | 24 | 33.33% | 1.79 | -1.10 | -26.45 | 139.50 |
| 7 | `strategy_dual_structure_trend` | 24 | 33.33% | 1.74 | -1.42 | -34.14 | 139.50 |
| 8 | `strategy_pullback_trend` | 17 | 17.65% | 3.49 | -2.79 | -47.48 | 149.00 |
| 9 | `strategy_vegas_martingale` | 12 | 25.00% | 1.36 | -4.03 | -48.34 | 55.29 |
| 10 | `strategy_squeeze_dual_structure` | 24 | 33.33% | 1.54 | -2.66 | -63.74 | 139.50 |
| 11 | `strategy_fib_martingale` | 23 | 47.83% | 0.78 | -2.82 | -64.76 | 203.23 |
| 12 | `strategy_price_action` | 24 | 25.00% | 2.19 | -3.22 | -77.26 | 128.33 |
| 13 | `strategy_fib_martingale_050` | 27 | 51.85% | 0.48 | -5.48 | -148.03 | 251.70 |
| 14 | `strategy_shadow_reversal` | 18 | 11.11% | 2.51 | -10.00 | -180.00 | 205.85 |
| 15 | `strategy_fib_martingale_1h` | 105 | 38.10% | 1.06 | -2.17 | -227.78 | 336.95 |
| 16 | `strategy_fib_martingale_1h_050` | 113 | 46.02% | 0.71 | -2.57 | -290.49 | 389.16 |

### 12 months (2025-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_dual_structure_trend` | 72 | 44.44% | 1.88 | 4.04 | **290.98** | 139.50 |
| 2 | `strategy_pullback_plus_breakout` | 72 | 43.06% | 1.95 | 3.75 | 269.81 | 139.50 |
| 3 | `strategy_pullback_continuation` | 34 | 55.88% | 1.86 | 7.78 | 264.37 | 79.31 |
| 4 | `strategy_squeeze_dual_structure` | 70 | 45.71% | 1.70 | 3.60 | 252.15 | 139.50 |
| 5 | `strategy_shadow_reversal` | 48 | 35.42% | 2.61 | 4.95 | 237.75 | 213.95 |
| 6 | `strategy_volume_breakout` | 43 | 44.19% | 1.59 | 4.09 | 175.88 | 284.74 |
| 7 | `strategy_rsi_ema` | 83 | 36.14% | 2.00 | 1.67 | 138.43 | 350.60 |
| 8 | `strategy_fib_martingale_050` | 59 | 59.32% | 0.79 | 1.59 | 93.95 | 251.70 |
| 9 | `strategy_pullback_trend` | 51 | 35.29% | 2.07 | 1.14 | 57.93 | 149.00 |
| 10 | `strategy_horizon_long` | 65 | 30.77% | 2.31 | 0.87 | 56.64 | 617.70 |
| 11 | `strategy_price_action` | 62 | 32.26% | 2.23 | 0.79 | 48.83 | 247.41 |
| 12 | `strategy_vegas_martingale` | 16 | 37.50% | 1.93 | 1.18 | 18.82 | 61.49 |
| 13 | `strategy_fib_martingale` | 51 | 49.02% | 1.06 | 0.17 | 8.57 | 203.23 |
| 14 | `strategy_fast` | 45 | 28.89% | 2.11 | -0.68 | -30.56 | 92.36 |
| 15 | `strategy_fib_martingale_1h_050` | 228 | 50.88% | 0.89 | -0.53 | -120.90 | 391.35 |
| 16 | `strategy_fib_martingale_1h` | 205 | 41.46% | 1.13 | -1.37 | -281.74 | 426.83 |

### 24 months (2024-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 137 | 32.12% | 2.41 | 5.07 | **694.60** | 1540.62 |
| 2 | `strategy_squeeze_dual_structure` | 151 | 49.67% | 1.37 | 3.44 | 520.11 | 234.58 |
| 3 | `strategy_rsi_ema` | 133 | 41.35% | 1.70 | 3.36 | 446.97 | 614.92 |
| 4 | `strategy_dual_structure_trend` | 145 | 47.59% | 1.44 | 2.95 | 428.09 | 254.06 |
| 5 | `strategy_volume_breakout` | 107 | 46.73% | 1.37 | 3.99 | 427.26 | 492.80 |
| 6 | `strategy_pullback_plus_breakout` | 140 | 46.43% | 1.46 | 2.64 | 368.90 | 260.69 |
| 7 | `strategy_pullback_continuation` | 70 | 57.14% | 1.23 | 4.77 | 333.83 | 105.61 |
| 8 | `strategy_price_action` | 128 | 37.50% | 1.95 | 2.53 | 324.21 | 331.48 |
| 9 | `strategy_shadow_reversal` | 110 | 31.82% | 2.54 | 2.63 | 288.99 | 314.55 |
| 10 | `strategy_pullback_trend` | 100 | 40.00% | 1.87 | 2.59 | 259.44 | 274.36 |
| 11 | `strategy_fib_martingale` | 101 | 49.50% | 1.17 | 1.99 | 200.89 | 313.10 |
| 12 | `strategy_fib_martingale_1h_050` | 437 | 51.49% | 0.95 | 0.10 | 43.09 | 481.33 |
| 13 | `strategy_vegas_martingale` | 32 | 34.38% | 1.90 | -0.03 | -0.84 | 145.65 |
| 14 | `strategy_fast` | 156 | 27.56% | 2.21 | -0.83 | -128.71 | 193.11 |
| 15 | `strategy_fib_martingale_050` | 115 | 52.17% | 0.84 | -1.29 | -148.81 | 498.74 |
| 16 | `strategy_fib_martingale_1h` | 398 | 43.47% | 1.14 | -0.93 | -370.32 | 494.51 |

### 36 months (2023-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 190 | 33.68% | 2.38 | 7.51 | **1426.04** | 1544.23 |
| 2 | `strategy_squeeze_dual_structure` | 245 | 49.39% | 1.39 | 3.15 | 770.79 | 234.58 |
| 3 | `strategy_dual_structure_trend` | 239 | 47.70% | 1.43 | 2.74 | 653.87 | 254.06 |
| 4 | `strategy_volume_breakout` | 168 | 48.21% | 1.29 | 3.70 | 622.18 | 492.79 |
| 5 | `strategy_pullback_plus_breakout` | 233 | 45.49% | 1.52 | 2.40 | 559.84 | 260.69 |
| 6 | `strategy_pullback_trend` | 147 | 40.82% | 1.78 | 2.33 | 342.25 | 274.36 |
| 7 | `strategy_pullback_continuation` | 119 | 52.94% | 1.16 | 2.34 | 278.20 | 205.67 |
| 8 | `strategy_fib_martingale` | 143 | 51.05% | 1.10 | 1.91 | 272.65 | 313.10 |
| 9 | `strategy_price_action` | 195 | 37.44% | 1.84 | 1.36 | 264.46 | 331.48 |
| 10 | `strategy_rsi_ema` | 106 | 41.51% | 1.55 | 1.74 | 184.43 | 400.79 |
| 11 | `strategy_vegas_martingale` | 37 | 37.84% | 2.47 | 3.44 | 127.36 | 94.95 |
| 12 | `strategy_shadow_reversal` | 150 | 30.00% | 2.45 | 0.70 | 104.73 | 314.55 |
| 13 | `strategy_fib_martingale_050` | 166 | 53.61% | 0.79 | -1.24 | -205.49 | 524.93 |
| 14 | `strategy_fast` | 192 | 26.04% | 2.22 | -1.12 | -215.16 | 243.22 |
| 15 | `strategy_fib_martingale_1h_050` | 637 | 51.65% | 0.87 | -0.55 | -347.93 | 629.73 |
| 16 | `strategy_fib_martingale_1h` | 623 | 45.26% | 1.05 | -0.89 | -552.29 | 704.13 |

### 48 months (2022-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 263 | 31.94% | 2.61 | 6.56 | **1724.22** | 1544.23 |
| 2 | `strategy_squeeze_dual_structure` | 327 | 47.71% | 1.50 | 2.73 | 892.28 | 234.58 |
| 3 | `strategy_dual_structure_trend` | 318 | 46.23% | 1.54 | 2.37 | 754.43 | 254.06 |
| 4 | `strategy_pullback_plus_breakout` | 312 | 44.23% | 1.62 | 2.10 | 656.30 | 260.69 |
| 5 | `strategy_volume_breakout` | 199 | 48.24% | 1.26 | 3.06 | 609.75 | 492.79 |
| 6 | `strategy_pullback_trend` | 210 | 40.00% | 1.87 | 2.00 | 419.08 | 274.36 |
| 7 | `strategy_price_action` | 242 | 38.84% | 1.78 | 1.55 | 375.82 | 331.48 |
| 8 | `strategy_pullback_continuation` | 144 | 53.47% | 1.16 | 2.24 | 322.38 | 205.67 |
| 9 | `strategy_fib_martingale` | 193 | 48.19% | 1.20 | 1.25 | 240.38 | 313.10 |
| 10 | `strategy_vegas_martingale` | 51 | 41.18% | 2.23 | 3.03 | 154.67 | 94.95 |
| 11 | `strategy_rsi_ema` | 158 | 39.24% | 1.63 | 0.68 | 107.92 | 400.79 |
| 12 | `strategy_shadow_reversal` | 151 | 29.80% | 2.46 | 0.63 | 94.93 | 314.55 |
| 13 | `strategy_fast` | 194 | 25.77% | 2.18 | -1.28 | -248.63 | 280.73 |
| 14 | `strategy_fib_martingale_050` | 209 | 51.20% | 0.86 | -1.37 | -286.13 | 597.83 |
| 15 | `strategy_fib_martingale_1h_050` | 832 | 51.68% | 0.87 | -0.46 | -381.15 | 629.73 |
| 16 | `strategy_fib_martingale_1h` | 805 | 45.09% | 1.06 | -0.77 | -617.50 | 704.13 |

### 60 months (2021-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 330 | 31.21% | 2.43 | 2.98 | **982.43** | 1544.23 |
| 2 | `strategy_volume_breakout` | 239 | 48.95% | 1.28 | 3.76 | 898.28 | 492.79 |
| 3 | `strategy_squeeze_dual_structure` | 406 | 47.29% | 1.43 | 2.00 | 810.78 | 234.58 |
| 4 | `strategy_dual_structure_trend` | 397 | 45.09% | 1.51 | 1.66 | 660.77 | 254.06 |
| 5 | `strategy_price_action` | 313 | 38.66% | 1.85 | 1.81 | 565.66 | 331.48 |
| 6 | `strategy_pullback_plus_breakout` | 391 | 42.71% | 1.60 | 1.38 | 541.44 | 260.69 |
| 7 | `strategy_rsi_ema` | 363 | 37.47% | 1.86 | 1.38 | 501.29 | 598.69 |
| 8 | `strategy_fib_martingale` | 235 | 48.94% | 1.18 | 1.40 | 328.98 | 313.10 |
| 9 | `strategy_pullback_trend` | 261 | 37.55% | 1.91 | 1.15 | 300.54 | 274.36 |
| 10 | `strategy_pullback_continuation` | 184 | 52.17% | 1.12 | 1.39 | 256.47 | 205.67 |
| 11 | `strategy_vegas_martingale` | 69 | 34.78% | 2.39 | 1.47 | 101.73 | 94.95 |
| 12 | `strategy_shadow_reversal` | 174 | 30.46% | 2.36 | 0.45 | 79.01 | 320.41 |
| 13 | `strategy_fast` | 182 | 26.92% | 2.22 | -0.93 | -168.41 | 724.92 |
| 14 | `strategy_fib_martingale_050` | 259 | 52.12% | 0.87 | -0.68 | -176.81 | 705.03 |
| 15 | `strategy_fib_martingale_1h_050` | 1039 | 52.55% | 0.87 | -0.20 | -203.10 | 735.23 |
| 16 | `strategy_fib_martingale_1h` | 1039 | 46.10% | 1.05 | -0.57 | -594.61 | 704.13 |

### 72 months (2020-11 to 2026-10)

| # | Strategy | Trades | Win rate | Profit/loss ratio | Average profit | Total profit | Max drawdown |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 398 | 31.16% | 2.77 | 7.58 | **3017.09** | 1544.23 |
| 2 | `strategy_squeeze_dual_structure` | 488 | 48.16% | 1.44 | 2.51 | 1223.25 | 234.58 |
| 3 | `strategy_dual_structure_trend` | 474 | 46.20% | 1.51 | 2.17 | 1029.30 | 254.06 |
| 4 | `strategy_volume_breakout` | 294 | 48.98% | 1.24 | 3.26 | 959.34 | 492.79 |
| 5 | `strategy_pullback_plus_breakout` | 467 | 44.11% | 1.61 | 1.99 | 931.40 | 260.69 |
| 6 | `strategy_rsi_ema` | 420 | 38.33% | 1.90 | 2.15 | 903.11 | 577.33 |
| 7 | `strategy_price_action` | 379 | 38.79% | 1.84 | 1.80 | 680.64 | 331.48 |
| 8 | `strategy_pullback_continuation` | 230 | 53.91% | 1.15 | 2.38 | 546.27 | 257.12 |
| 9 | `strategy_fib_martingale` | 279 | 48.39% | 1.23 | 1.58 | 440.55 | 313.10 |
| 10 | `strategy_pullback_trend` | 305 | 37.05% | 1.91 | 1.01 | 307.47 | 274.36 |
| 11 | `strategy_vegas_martingale` | 76 | 34.21% | 2.49 | 1.67 | 127.03 | 116.34 |
| 12 | `strategy_fib_martingale_050` | 303 | 52.48% | 0.86 | -0.61 | -183.62 | 705.03 |
| 13 | `strategy_shadow_reversal` | 228 | 28.95% | 2.22 | -1.37 | -311.41 | 654.36 |
| 14 | `strategy_fib_martingale_1h_050` | 1244 | 52.57% | 0.85 | -0.34 | -425.04 | 776.82 |
| 15 | `strategy_fib_martingale_1h` | 1203 | 46.63% | 1.07 | -0.38 | -453.10 | 784.14 |
| 16 | `strategy_fast` | 159 | 27.04% | 1.55 | -3.18 | -506.33 | 766.27 |



### Observations

1. **`dual_structure_trend` wins at 12 months (+290.98), while `horizon_long` wins at 24/36/48/60/72** (+694.60 / +1426.04 / +1724.22 / +982.43 / +3017.09). Its 1544.23 drawdown is the largest in the table though — the 72-month profit is only about twice the drawdown.
2. **`squeeze_dual_structure` has the flattest risk profile**: its drawdown stays at 234.58 for every window from 24 months up, while profit keeps growing to +1223.25 over 72 months — longer windows raise the return without raising the risk.
3. **`horizon_long` makes only +982.43 over 60 months, less than the +1724.22 it makes over 48**: those extra twelve months were a net loss, so it lives off a few trending stretches rather than steady income.
4. **`pullback_continuation` trades the least and draws down the least**: 34 trades and a 79.31 drawdown at 12 months (+264.37); even at 72 months it is only 230 trades with a 257.12 drawdown.
5. **Short windows carry almost no information**: only 4 strategies are positive at 3 months and 3 at 6 months. `horizon_long` tops both, which is exactly why its profit is clearly concentrated in the recent stretch.
6. **`fib_martingale` barely works even on 4h** (+440.55 over 72 months); the 1h variants lose in all eight windows and the `_050` variant turns negative after 24 months.
7. **`fast` and `shadow_reversal` fall apart on long windows**: `fast` slides from −30.56 to −506.33, and `shadow_reversal` ends at −311.41 over 72 months.
