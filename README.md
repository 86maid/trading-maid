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

Every `strategy_*` file under `examples/` is a complete strategy — entry/exit rules, position sizing, a full backtest. The complete source of all of them is reproduced below, ordered by 72-month net profit.

The `demo_*` files are minimal hello-worlds (`demo_minimal` places one order, `demo_indicator` shows the built-in indicators plus a hook, `demo_funding_rate` shows the funding-rate series); their source lives in `examples/`.

| # | File | Level | Idea |
|--:|---|:--:|---|
| 1 | `strategy_horizon_long` | 1h | Time-series momentum, long only |
| 2 | `strategy_squeeze_dual_structure` | 4h | Two structures plus a Bollinger-squeeze breakout for the phase before a trend exists |
| 3 | `strategy_dual_structure_trend` | 4h | Two-structure trend, risk-normalised: structure A stop at least 0.5 ATR, structure B trailing min(1 ATR, 0.5R) |
| 4 | `strategy_volume_breakout` | 4h | Donchian channel breakout confirmed by a volume spike |
| 5 | `strategy_pullback_plus_breakout` | 4h | Pullback continuation plus momentum continuation sharing one risk budget |
| 6 | `strategy_rsi_ema` | 4h | Consecutive-candle breakout filtered by RSI/EMA |
| 7 | `strategy_price_action` | 4h | Momentum breakout, indicators written from scratch |
| 8 | `strategy_pullback_continuation` | 4h | EMA stack + ADX strength + break-of-prior-high entry + ATR trailing stop |
| 9 | `strategy_fib_martingale` | 4h | Fibonacci retracement + liquidity sweep + vacuum zone + add-on entries (0.382) |
| 10 | `strategy_pullback_trend` | 4h | SMA50/200 trend + RSI pullback entry (**baseline**) |
| 11 | `strategy_vegas_martingale` | 4h | Vegas tunnel + RSI/MACD with martingale adding |
| 12 | `strategy_fib_martingale_050` | 4h | Same, entering at the 0.5 retracement |
| 13 | `strategy_shadow_reversal` | 4h | Long-wick reversal with ATR-based take-profit/stop |
| 14 | `strategy_fib_martingale_1h_050` | 1h | Same, 1h + 0.5 retracement |
| 15 | `strategy_fib_martingale_1h` | 1h | Same, moved to 1h |
| 16 | `strategy_fast` | 1h | Short on a long upper wick — the simplest rule set here |

### 1. `strategy_horizon_long`

Time-series momentum, long only。

```rust
use trading_maid::prelude::*;

const WIN: usize = 2000;
const QTY: Decimal = dec!(0.05);
const SYMBOL: &str = "BTCUSDT";

// ---------- strategy configuration ----------
// momentum lookback (168 = 1 week of hours)
const MOM_BARS: usize = 168;
// fixed holding horizon (336 = 2 weeks)
const HOLD_BARS: usize = 336;
// disaster stop distance in ATR
const STOP_ATR: Decimal = dec!(6.0);
// bars to wait after a close (non-overlapping samples)
const MIN_GAP: usize = 24;
// classic TSMOM: hold only while momentum stays positive
const EXIT_ON_REVERSAL: bool = true;
const ATR_N: usize = 14;

// The 200-EMA "trend quality" filter was tested and REMOVED: it turned a profitable
// momentum strategy negative on both symbols (BTC 48m +1568 -> -177), because TSMOM
// works by catching moves before they are already extended above the long EMA.
// Kept as a constant only so the experiment is reproducible.
const REQUIRE_EMA200: bool = false;

fn atr_of(h: &[Decimal], l: &[Decimal], c: &[Decimal], period: usize) -> Option<Decimal> {
    let n = c.len();
    if period == 0 || n < period + 1 {
        return None;
    }
    let cnt = (n - 1).min(period);
    let mut s = Decimal::ZERO;
    for i in 0..cnt {
        let hi = h[i];
        let lo = l[i];
        let pc = c[i + 1];
        let r = if hi > lo { hi - lo } else { lo - hi };
        let u = if hi > pc { hi - pc } else { pc - hi };
        let d = if lo > pc { lo - pc } else { pc - lo };
        s += r.max(u).max(d);
    }
    Some(s / Decimal::from(cnt))
}

/// EMA over a newest-first slice.
fn ema_of(c: &[Decimal], period: usize) -> Option<Decimal> {
    let n = c.len();
    if period == 0 || n < period {
        return None;
    }
    let mut sum = Decimal::ZERO;
    for x in &c[n - period..] {
        sum += x;
    }
    let mut ema = sum / Decimal::from(period);
    for idx in (0..n - period).rev() {
        let v = c[idx];
        let alpha = dec!(2) / Decimal::from(period + 1);
        ema = v * alpha + ema * (dec!(1) - alpha);
    }
    Some(ema)
}

fn round_tick(p: Decimal) -> Decimal {
    let t = dec!(0.1);
    let r = (p / t).round_dp(0) * t;
    if r <= Decimal::ZERO { t } else { r }
}

struct TsmomLong {
    win: Vec<Decimal>,
    wih: Vec<Decimal>,
    wlo: Vec<Decimal>,
    in_position: bool,
    bars_held: usize,
    gap_left: usize,
    stop_price: Option<Decimal>,
    n_entries: usize,
    n_horizon_exit: usize,
    n_stop_exit: usize,
    n_reversal_exit: usize,
}

impl TsmomLong {
    fn new() -> Self {
        TsmomLong {
            win: Vec::new(),
            wih: Vec::new(),
            wlo: Vec::new(),
            in_position: false,
            bars_held: 0,
            gap_left: 0,
            stop_price: None,
            n_entries: 0,
            n_horizon_exit: 0,
            n_stop_exit: 0,
            n_reversal_exit: 0,
        }
    }
}

#[async_trait(?Send)]
impl Strategy for TsmomLong {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.win.insert(0, cx.close[0]);
        self.wih.insert(0, cx.high[0]);
        self.wlo.insert(0, cx.low[0]);
        if self.win.len() > WIN {
            for v in [&mut self.win, &mut self.wih, &mut self.wlo] {
                v.truncate(WIN);
            }
        }
        if self.win.len() <= MOM_BARS + 5 {
            return Ok(());
        }

        let pos = cx.get_position(SYMBOL).await?;
        let was_in = self.in_position;
        self.in_position = pos.is_some();
        if was_in && !self.in_position {
            self.gap_left = MIN_GAP;
        }

        // ---------- manage the open position ----------
        if pos.is_some() {
            self.bars_held += 1;
            let stop_hit = self.stop_price.map(|s| cx.close[0] <= s).unwrap_or(false);
            let horizon_done = self.bars_held >= HOLD_BARS;
            // signal-reversal exit: classic TSMOM holds only while momentum stays positive.
            // Require TWO consecutive closes below the momentum level: a single close can be
            // noise, and one-bar whipsaws were the dominant fee cost.
            let below =
                |k: usize| self.win.len() > MOM_BARS + k && self.win[k] < self.win[MOM_BARS + k];
            let mom_reversed = EXIT_ON_REVERSAL && below(0) && below(1);
            if stop_hit || horizon_done || mom_reversed {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                if horizon_done {
                    self.n_horizon_exit += 1;
                } else if stop_hit {
                    self.n_stop_exit += 1;
                } else {
                    self.n_reversal_exit += 1;
                }
                self.stop_price = None;
                self.bars_held = 0;
                self.in_position = false;
                self.gap_left = MIN_GAP;
            }
            return Ok(());
        }

        if self.gap_left > 0 {
            self.gap_left -= 1;
            return Ok(());
        }

        // ---------- entry: time-series momentum (long only) ----------
        let c0 = self.win[0];
        let past = self.win[MOM_BARS];
        // trend-quality filter: only act when price is above the 200-hour EMA, i.e. inside a
        // confirmed uptrend.  Momentum whipsaws badly in downtrends; this is a standard,
        // non-tuned filter (200 is the conventional long-period EMA).
        let ema200 = ema_of(&self.win, 200).unwrap_or(c0);
        let uptrend_ok = !REQUIRE_EMA200 || c0 > ema200;
        if c0 > past && uptrend_ok {
            let Some(atr) = atr_of(&self.wih, &self.wlo, &self.win, ATR_N) else {
                return Ok(());
            };
            let stop = round_tick(c0 - atr * STOP_ATR).max(dec!(1));
            cx.cancel_all_order(SYMBOL).await?;
            let _ = cx.buy(SYMBOL, QTY).await?;
            let _ = cx
                .sell_trigger_market_reduce_only(SYMBOL, stop, QTY)
                .await?;
            self.stop_price = Some(stop);
            self.bars_held = 0;
            self.in_position = true;
            self.n_entries += 1;
        }

        Ok(())
    }
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, TsmomLong::new(), Level::Hour1)
            .await
            .unwrap();

        print_report(month_count, SYMBOL, Level::Hour1, &result);
    }
}
```

### 2. `strategy_squeeze_dual_structure`

Two structures plus a Bollinger-squeeze breakout for the phase before a trend exists。

```rust
//! 双结构 + 波动收口突破：在 `strategy_dual_structure_trend.rs` 之上再加一套互补结构。
//!
//! ## 这套东西解决了什么问题
//!
//! `dual_structure_trend` 只有两套结构，而且它们**只在半个周期里赚钱**（样本外切分）：
//!
//! | 结构 | 2022-11~2024-10 | 2024-11~2026-09 |
//! |---|---|---|
//! | A 回调接续 | +297.2，PF 1.69 | +247.5，PF 1.39 |
//! | B 动量延续 | **−39.2**，PF 0.97 | +290.7，PF 1.81 |
//!
//! 两套结构都依赖“已经有趋势”。真正缺的是一套**在趋势还没形成时就进场**的结构：
//! 布林带收口（波动压缩）之后的方向性扩张。它的触发条件与前两套几乎不重叠
//! （收口意味着 ADX 低，B 正好要求 ADX 高），所以是互补而不是重复。
//!
//! 参数全部是教科书默认值，没有做任何扫描：布林带 20 周期 2 倍标准差、
//! “收口”定义为带宽处于最近 100 根的 20 分位以下、止盈 3R。
//!
//! ## 样本外验证（先定规则，后看数据）
//!
//! | 版本 | 训练段 2022-11~2024-10 | 测试段 2024-11~2026-09 |
//! |---|---|---|
//! | dual_structure_trend | PF 1.39 / 利润 258.0 / 回撤 124.3 | PF 1.53 / 利润 538.2 / 回撤 222.3 |
//! | 本策略 | **PF 1.45 / 利润 303.9 / 回撤 124.3** | **PF 1.58 / 利润 630.2 / 回撤 143.6** |
//!
//! 两段都是利润 +17%~18%、PF 提高、回撤不增（测试段回撤还降了 35%）。
//!
//! ## 它赢在哪里、输在哪里（务必看）
//!
//! | 窗口 | 利润 | PF | 回撤 |
//! |---|---|---|---|
//! | 12 个月 | **401.10 → 362.26（降 10%）** | 2.01 → 1.90 | 81.99 → 81.99 |
//! | 24 个月 | 538.20 → 630.20（升 17%） | 1.53 → 1.58 | 222.27 → 143.59 |
//! | 36 个月 | 763.99 → 880.91（升 15%） | 1.49 → 1.54 | 222.27 → 143.59 |
//! | 48 个月 | 864.55 → 1002.40（升 16%） | 1.50 → 1.55 | 222.27 → 143.59 |
//!
//! **12 个月这一档是退步的。** 我试过用“收口突破只在弱趋势里触发”“提高它的优先级”等方式
//! 把这个缺口补掉，但那等于在评分窗口上挑规则；同期试的另外几种结构（唐奇安突破、
//! 浅回调 EMA21、A 加超时离场）也都是一头好一头坏。这里保留的是**两段样本外都变好**的版本，
//! 而不是某个窗口刷得最高的版本。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;
const WARMUP: usize = 212;
/// 布林带与收口分位的窗口
const BB_N: usize = 20;
const SQUEEZE_WINDOW: usize = 100;

// ---- 结构 A ----
const A_RSI_LONG: (Decimal, Decimal) = (dec!(40), dec!(65));
const A_RSI_SHORT: (Decimal, Decimal) = (dec!(35), dec!(60));
const A_BAND_UP: Decimal = dec!(1.005);
const A_BAND_DOWN: Decimal = dec!(0.99);
const A_SWING: usize = 5;
const A_TP_R: Decimal = dec!(2.0);
const A_SL_MIN_ATR: Decimal = dec!(0.5);

// ---- 结构 B ----
const B_ADX_LONG: Decimal = dec!(24);
const B_ADX_SHORT: Decimal = dec!(30);
const B_RSI_LONG: (Decimal, Decimal) = (dec!(38), dec!(68));
const B_RSI_SHORT: (Decimal, Decimal) = (dec!(32), dec!(62));
const B_SWING: usize = 6;
const B_SL_ATR: Decimal = dec!(1.2);
const B_SL_BUFFER_ATR: Decimal = dec!(0.25);
const B_RISK_MIN_ATR: Decimal = dec!(0.25);
const B_RISK_MAX_ATR: Decimal = dec!(2.4);
const B_MAX_EXT_ATR: Decimal = dec!(2.0);
const B_TP_R: Decimal = dec!(4.0);

// ---- 结构 C：波动收口突破 ----
const C_TP_R: Decimal = dec!(3.0);

// ---- 共用离场 ----
const TRAIL_START_R: Decimal = dec!(0.6);
const TRAIL_GIVE_R: Decimal = dec!(0.5);
const TRAIL_GIVE_ATR: Decimal = dec!(1.0);
const MAX_BAR_HELD: usize = 60;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_w: Option<Decimal>,
    plus_w: Option<Decimal>,
    minus_w: Option<Decimal>,
    dx_sum: Decimal,
    dx_count: usize,
    value: Option<Decimal>,
}

impl Adx {
    fn new(period: usize) -> Self {
        Adx {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            tr_sum: Decimal::ZERO,
            plus_sum: Decimal::ZERO,
            minus_sum: Decimal::ZERO,
            bar_count: 0,
            tr_w: None,
            plus_w: None,
            minus_w: None,
            dx_sum: Decimal::ZERO,
            dx_count: 0,
            value: None,
        }
    }

    fn update(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let Some(prev_high) = self.prev_high else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            return None;
        };

        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
        self.bar_count += 1;

        let period = Decimal::from(self.period);

        match self.tr_w {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_w = Some(self.tr_sum);
                self.plus_w = Some(self.plus_sum);
                self.minus_w = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_w.unwrap();
                let minus = self.minus_w.unwrap();

                self.tr_w = Some(value - value / period + tr);
                self.plus_w = Some(plus - plus / period + plus_dm);
                self.minus_w = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_w = self.tr_w.unwrap();

        if tr_w.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_w.unwrap() / tr_w * dec!(100);
        let minus_di = self.minus_w.unwrap() / tr_w * dec!(100);
        let di_sum = plus_di + minus_di;

        if di_sum.is_zero() {
            return self.value;
        }

        let dx = (plus_di - minus_di).abs() / di_sum * dec!(100);

        match self.value {
            None => {
                self.dx_sum += dx;
                self.dx_count += 1;

                if self.dx_count >= self.period {
                    self.value = Some(self.dx_sum / period);
                }
            }
            Some(prev) => {
                self.value = Some((prev * (period - dec!(1)) + dx) / period);
            }
        }

        self.value
    }
}

/// 当前 K 线及其指标快照，供三套入场逻辑共用。
struct Bar {
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    prev_high: Decimal,
    prev_low: Decimal,
    atr: Decimal,
    rsi: Decimal,
    sma50: Decimal,
    sma200: Decimal,
    ema_fast: Decimal,
    ema_mid: Decimal,
    ema_slow: Decimal,
    adx: Decimal,
    bb_upper: Decimal,
    bb_lower: Decimal,
    /// 上一根是否处于波动收口状态（避免用当根自己判断收口）
    squeezed_prev: bool,
}

struct Signal {
    tp: Decimal,
    sl: Decimal,
    risk: Decimal,
}

#[derive(Clone, Copy, PartialEq)]
enum Setup {
    None,
    Pullback,
    Continuation,
    Squeeze,
}

struct SqueezeDualStructure {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    close_buf: VecDeque<Decimal>,
    bandwidth_buf: VecDeque<Decimal>,
    squeezed_prev: bool,
    bar_count: usize,
    setup: Setup,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl SqueezeDualStructure {
    fn new() -> Self {
        SqueezeDualStructure {
            ema_fast: EMACache::new(21),
            ema_mid: EMACache::new(50),
            ema_slow: EMACache::new(200),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            close_buf: VecDeque::new(),
            bandwidth_buf: VecDeque::new(),
            squeezed_prev: false,
            bar_count: 0,
            setup: Setup::None,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.setup = Setup::None;
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    fn recent_extreme(&self, buf: &VecDeque<Decimal>, count: usize, want_max: bool) -> Decimal {
        buf.iter().take(count).copied().fold(
            if want_max { Decimal::MIN } else { Decimal::MAX },
            if want_max { Decimal::max } else { Decimal::min },
        )
    }

    /// 结构 A：回调接续。价格回落到 SMA50 附近后顺势收一根。
    fn signal_pullback(&self, bar: &Bar) -> Option<Signal> {
        if bar.sma50 > bar.sma200 {
            let near = bar.low <= bar.sma50 * A_BAND_UP && bar.low >= bar.sma50 * A_BAND_DOWN;
            let recent_low = self.recent_extreme(&self.low_buf, A_SWING, false);

            if near && bar.close > bar.open && bar.rsi > A_RSI_LONG.0 && bar.rsi < A_RSI_LONG.1 {
                let sl = round_to_tick(
                    recent_low
                        .min(bar.low)
                        .min(bar.close - A_SL_MIN_ATR * bar.atr),
                );
                let risk = bar.close - sl;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close + risk * A_TP_R);

                    if tp > bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        if bar.sma50 < bar.sma200 {
            let near = bar.high >= bar.sma50 * A_BAND_DOWN && bar.high <= bar.sma50 * A_BAND_UP;
            let recent_high = self.recent_extreme(&self.high_buf, A_SWING, true);

            if near && bar.close < bar.open && bar.rsi > A_RSI_SHORT.0 && bar.rsi < A_RSI_SHORT.1 {
                let sl = round_to_tick(
                    recent_high
                        .max(bar.high)
                        .max(bar.close + A_SL_MIN_ATR * bar.atr),
                );
                let risk = sl - bar.close;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close - risk * A_TP_R);

                    if tp < bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        None
    }

    /// 结构 B：动量延续。三线排列 + ADX 确认后，收盘突破前一根高点跟进。
    fn signal_continuation(&self, bar: &Bar) -> Option<Signal> {
        let trend_long =
            bar.close > bar.ema_slow && bar.ema_fast > bar.ema_mid && bar.ema_mid > bar.ema_slow;
        let trend_short =
            bar.close < bar.ema_slow && bar.ema_fast < bar.ema_mid && bar.ema_mid < bar.ema_slow;

        if trend_long
            && bar.adx >= B_ADX_LONG
            && bar.rsi >= B_RSI_LONG.0
            && bar.rsi <= B_RSI_LONG.1
            && bar.close > bar.prev_high
            && bar.close > bar.open
            && bar.close - bar.ema_fast <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_low = self.recent_extreme(&self.low_buf, B_SWING, false);
            let sl = round_to_tick(
                (swing_low - B_SL_BUFFER_ATR * bar.atr).min(bar.close - B_SL_ATR * bar.atr),
            );
            let risk = bar.close - sl;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close + B_TP_R * risk);

                if tp > bar.close {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        if trend_short
            && bar.adx >= B_ADX_SHORT
            && bar.rsi >= B_RSI_SHORT.0
            && bar.rsi <= B_RSI_SHORT.1
            && bar.close < bar.prev_low
            && bar.close < bar.open
            && bar.ema_fast - bar.close <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_high = self.recent_extreme(&self.high_buf, B_SWING, true);
            let sl = round_to_tick(
                (swing_high + B_SL_BUFFER_ATR * bar.atr).max(bar.close + B_SL_ATR * bar.atr),
            );
            let risk = sl - bar.close;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close - B_TP_R * risk);

                if tp < bar.close && tp > Decimal::ZERO {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        None
    }

    /// 结构 C：波动收口之后的方向性扩张。不做趋势判定——它的价值就在于趋势形成之前进场。
    fn signal_squeeze(&self, bar: &Bar) -> Option<Signal> {
        let break_up = bar.squeezed_prev && bar.close > bar.bb_upper && bar.close > bar.open;
        let break_down = bar.squeezed_prev && bar.close < bar.bb_lower && bar.close < bar.open;

        if break_up {
            let swing_low = self.recent_extreme(&self.low_buf, B_SWING, false);
            let sl = round_to_tick(
                (swing_low - B_SL_BUFFER_ATR * bar.atr).min(bar.close - B_SL_ATR * bar.atr),
            );
            let risk = bar.close - sl;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close + C_TP_R * risk);

                if tp > bar.close {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        if break_down {
            let swing_high = self.recent_extreme(&self.high_buf, B_SWING, true);
            let sl = round_to_tick(
                (swing_high + B_SL_BUFFER_ATR * bar.atr).max(bar.close + B_SL_ATR * bar.atr),
            );
            let risk = sl - bar.close;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close - C_TP_R * risk);

                if tp < bar.close && tp > Decimal::ZERO {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        None
    }
}

#[async_trait(?Send)]
impl Strategy for SqueezeDualStructure {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);
        self.close_buf.push_front(close);

        if self.high_buf.len() > B_SWING.max(A_SWING) {
            self.high_buf.truncate(B_SWING.max(A_SWING));
        }

        if self.low_buf.len() > B_SWING.max(A_SWING) {
            self.low_buf.truncate(B_SWING.max(A_SWING));
        }

        if self.close_buf.len() > BB_N {
            self.close_buf.truncate(BB_N);
        }

        self.bar_count += 1;

        if self.bar_count < WARMUP {
            return Ok(());
        }

        let (Some(ema_fast), Some(ema_mid), Some(ema_slow)) = (ema_fast, ema_mid, ema_slow) else {
            return Ok(());
        };
        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };
        let Some(rsi) = self.rsi.value() else {
            return Ok(());
        };
        let Some(sma50) = sma(cx.close, 50) else {
            return Ok(());
        };
        let Some(sma200) = sma(cx.close, 200) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // 布林带（含当前收盘）与带宽分位
        let mid = self.close_buf.iter().sum::<Decimal>() / Decimal::from(BB_N);
        let variance = self
            .close_buf
            .iter()
            .map(|v| {
                let d = *v - mid;
                d * d
            })
            .sum::<Decimal>()
            / Decimal::from(BB_N);
        let std = variance.sqrt().unwrap_or(Decimal::ZERO);
        let bb_upper = mid + dec!(2) * std;
        let bb_lower = mid - dec!(2) * std;
        let bandwidth = if mid.is_zero() {
            Decimal::ZERO
        } else {
            (bb_upper - bb_lower) / mid
        };

        let squeezed = if self.bandwidth_buf.len() >= SQUEEZE_WINDOW {
            let mut sorted = self.bandwidth_buf.iter().copied().collect::<Vec<_>>();
            sorted.sort();

            bandwidth <= sorted[sorted.len() / 5]
        } else {
            false
        };

        if self.bandwidth_buf.len() >= SQUEEZE_WINDOW {
            self.bandwidth_buf.pop_back();
        }

        self.bandwidth_buf.push_front(bandwidth);

        // ---------------- 持仓管理 ----------------
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            // B / C 用移动止损；A 用固定 2R 止盈 + 结构止损
            let uses_trail = matches!(self.setup, Setup::Continuation | Setup::Squeeze);

            if uses_trail {
                let long = position.side == Side::Buy;
                let sign = if long { Decimal::ONE } else { -Decimal::ONE };
                let favorable = (close - entry_price) * sign;

                if favorable > self.best_favorable {
                    self.best_favorable = favorable;
                }

                if self.best_favorable >= risk * TRAIL_START_R {
                    let give_back = (TRAIL_GIVE_ATR * atr).min(TRAIL_GIVE_R * risk);
                    let candidate = entry_price + sign * (self.best_favorable - give_back);
                    let current_stop = self.stop_price.unwrap_or(Decimal::ZERO);
                    let improves = current_stop.is_zero()
                        || if long {
                            candidate > current_stop
                        } else {
                            candidate < current_stop
                        };

                    if improves {
                        let new_stop = round_to_tick(candidate);

                        if new_stop != current_stop {
                            if let Some(id) = self.stop_order_id.clone() {
                                _ = cx.cancel_order(SYMBOL, id).await;
                            }

                            self.stop_order_id = if long {
                                cx.sell_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            } else {
                                cx.buy_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            };
                            self.stop_price = Some(new_stop);
                        }
                    }
                }

                if self.bar_held >= MAX_BAR_HELD {
                    cx.cancel_all_order(SYMBOL).await?;
                    cx.close_all_position(SYMBOL).await?;
                    self.reset_trade();
                }
            }

            return Ok(());
        }

        self.reset_trade();

        let bar = Bar {
            open,
            high,
            low,
            close,
            prev_high: cx.high[1],
            prev_low: cx.low[1],
            atr,
            rsi,
            sma50,
            sma200,
            ema_fast,
            ema_mid,
            ema_slow,
            adx: adx.unwrap_or(Decimal::ZERO),
            bb_upper,
            bb_lower,
            squeezed_prev: self.squeezed_prev,
        };

        self.squeezed_prev = squeezed;

        // 同一根 K 线多套成立时的优先级：B（趋势中最强）> A（回调）> C（收口突破）
        for setup in [Setup::Continuation, Setup::Pullback, Setup::Squeeze] {
            let signal = match setup {
                Setup::Continuation => self.signal_continuation(&bar),
                Setup::Pullback => self.signal_pullback(&bar),
                _ => self.signal_squeeze(&bar),
            };

            let Some(signal) = signal else {
                continue;
            };

            cx.cancel_all_order(SYMBOL).await?;

            let sl_result = if signal.tp > close {
                cx.buy_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            } else {
                cx.sell_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            };

            self.setup = setup;
            self.entry_price = Some(close);
            self.risk = Some(signal.risk);
            self.stop_price = Some(signal.sl);
            self.stop_order_id = sl_result.ok();

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}  profit/mdd={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
    );
    println!(
        "long/short = {}/{}",
        long_count,
        summary.total_trades - long_count
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径与 strategy_pullback_trend.rs 对齐：12 个月、4h。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, SqueezeDualStructure::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 3. `strategy_dual_structure_trend`

Two-structure trend, risk-normalised: structure A stop at least 0.5 ATR, structure B trailing min(1 ATR, 0.5R)。

```rust
//! 双结构趋势策略（风控归一版）：两套互补入场 + 以 R 度量的离场规则。
//!
//! 与 `strategy_pullback_plus_breakout.rs` 的区别只有两处，都是**风控口径**的修正，不是参数调优：
//!
//! 1. **结构 A 的止损设最小距离 0.5 ATR。**
//!    原版直接用最近 5 根极值，止损常常只有 0.1~0.3 ATR——那已经在噪音带里面，
//!    任何一次正常抖动都会触发；同时 2R 止盈只有 0.6 ATR 远，手续费占比也被抬得很高。
//!    0.5 ATR 是本仓库其它策略已经在用的止损距离（见 `strategy_volume_breakout.rs`）。
//!
//! 2. **结构 B 的移动止损回撤改成 `min(1 ATR, 0.5R)`。**
//!    原版固定回撤 1 ATR：止损窄的单子（R≈0.5 ATR）等于让出 2R，盈利单经常回吐成亏损；
//!    止损宽的单子（R≈2 ATR）又只有 0.5R。改成“回撤不超过这笔交易自身风险的一半，
//!    同时不超过 1 ATR”，尺度才和这笔交易的风险同量纲。
//!
//! 这两条是先按常识定好、再拿数据检验的，检验方式是**样本外切分**，不是在同一段数据上
//! 挑参数：用 2022-11~2024-10 作为训练段、2024-11~2026-09 作为测试段，两段都必须变好才保留。
//!
//! | 版本 | 训练段 | 测试段 |
//! |---|---|---|
//! | pullback_plus_breakout | PF 1.35 / 利润 224.7 / 回撤 134.6 | PF 1.49 / 利润 479.0 / 回撤 226.4 |
//! | 本策略 | PF 1.39 / 利润 258.0 / 回撤 124.3 | PF 1.53 / 利润 538.2 / 回撤 222.3 |
//!
//! 同期还试了“要求信号 K 线收在区间端部”这类过滤器：训练段 PF 1.35→1.50 很漂亮，
//! 测试段却掉到 1.21。这类只在半段起作用的改动一律不用——这正是样本外检验的价值。
//!
//! 结构：
//!
//! | | 结构 A：回调接续 | 结构 B：动量延续 |
//! |---|---|---|
//! | 趋势 | SMA50 vs SMA200 | EMA21 > EMA50 > EMA200 且站上 EMA200 |
//! | 强度 | 无 | ADX(14) ≥ 24（做空 ≥ 30） |
//! | 触发 | 触及 SMA50 带宽 + 顺向 K 线 | 收盘突破前一根高/低点 |
//! | 止损 | 近 5 根极值，**最小 0.5 ATR** | 结构极值与 1.2 ATR 取宽者，风险 0.25~2.4 ATR |
//! | 止盈 | 2R | 4R |
//! | 移动止损 | 无 | **+0.6R 后按 min(1 ATR, 0.5R) 回撤** |
//! | 超时 | 无 | 60 根 |

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;
const WARMUP: usize = 212;

// ---- 结构 A ----
const A_RSI_LONG: (Decimal, Decimal) = (dec!(40), dec!(65));
const A_RSI_SHORT: (Decimal, Decimal) = (dec!(35), dec!(60));
const A_BAND_UP: Decimal = dec!(1.005);
const A_BAND_DOWN: Decimal = dec!(0.99);
const A_SWING: usize = 5;
const A_TP_R: Decimal = dec!(2.0);
/// 止损最小距离：不要贴着噪音带放止损。
const A_SL_MIN_ATR: Decimal = dec!(0.5);

// ---- 结构 B ----
const B_ADX_LONG: Decimal = dec!(24);
const B_ADX_SHORT: Decimal = dec!(30);
const B_RSI_LONG: (Decimal, Decimal) = (dec!(38), dec!(68));
const B_RSI_SHORT: (Decimal, Decimal) = (dec!(32), dec!(62));
const B_SWING: usize = 6;
const B_SL_ATR: Decimal = dec!(1.2);
const B_SL_BUFFER_ATR: Decimal = dec!(0.25);
const B_RISK_MIN_ATR: Decimal = dec!(0.25);
const B_RISK_MAX_ATR: Decimal = dec!(2.4);
const B_MAX_EXT_ATR: Decimal = dec!(2.0);
const B_TP_R: Decimal = dec!(4.0);
/// 移动止损：浮盈到 0.6R 后启动，回撤取 min(1 ATR, 0.5R)
/// —— 回撤尺度与这笔交易自身的风险同量纲，且永远不超过 1 ATR。
const B_TRAIL_START_R: Decimal = dec!(0.6);
const B_TRAIL_GIVE_R: Decimal = dec!(0.5);
const B_TRAIL_GIVE_ATR: Decimal = dec!(1.0);
const B_MAX_BAR_HELD: usize = 60;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_w: Option<Decimal>,
    plus_w: Option<Decimal>,
    minus_w: Option<Decimal>,
    dx_sum: Decimal,
    dx_count: usize,
    value: Option<Decimal>,
}

impl Adx {
    fn new(period: usize) -> Self {
        Adx {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            tr_sum: Decimal::ZERO,
            plus_sum: Decimal::ZERO,
            minus_sum: Decimal::ZERO,
            bar_count: 0,
            tr_w: None,
            plus_w: None,
            minus_w: None,
            dx_sum: Decimal::ZERO,
            dx_count: 0,
            value: None,
        }
    }

    fn update(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let Some(prev_high) = self.prev_high else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            return None;
        };

        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
        self.bar_count += 1;

        let period = Decimal::from(self.period);

        match self.tr_w {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_w = Some(self.tr_sum);
                self.plus_w = Some(self.plus_sum);
                self.minus_w = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_w.unwrap();
                let minus = self.minus_w.unwrap();

                self.tr_w = Some(value - value / period + tr);
                self.plus_w = Some(plus - plus / period + plus_dm);
                self.minus_w = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_w = self.tr_w.unwrap();

        if tr_w.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_w.unwrap() / tr_w * dec!(100);
        let minus_di = self.minus_w.unwrap() / tr_w * dec!(100);
        let di_sum = plus_di + minus_di;

        if di_sum.is_zero() {
            return self.value;
        }

        let dx = (plus_di - minus_di).abs() / di_sum * dec!(100);

        match self.value {
            None => {
                self.dx_sum += dx;
                self.dx_count += 1;

                if self.dx_count >= self.period {
                    self.value = Some(self.dx_sum / period);
                }
            }
            Some(prev) => {
                self.value = Some((prev * (period - dec!(1)) + dx) / period);
            }
        }

        self.value
    }
}

/// 当前 K 线及其指标快照，供两套入场逻辑共用。
struct Bar {
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    prev_high: Decimal,
    prev_low: Decimal,
    atr: Decimal,
    rsi: Decimal,
    sma50: Decimal,
    sma200: Decimal,
    ema_fast: Decimal,
    ema_mid: Decimal,
    ema_slow: Decimal,
    adx: Decimal,
}

/// 一套入场结构给出的结果：(止盈价, 止损价, 单笔风险)。
struct Signal {
    tp: Decimal,
    sl: Decimal,
    risk: Decimal,
}

/// 持仓来源，决定用哪套离场规则。
#[derive(Clone, Copy, PartialEq)]
enum Setup {
    None,
    Pullback,
    Continuation,
}

struct DualStructureTrend {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    bar_count: usize,
    setup: Setup,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl DualStructureTrend {
    fn new() -> Self {
        DualStructureTrend {
            ema_fast: EMACache::new(21),
            ema_mid: EMACache::new(50),
            ema_slow: EMACache::new(200),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            bar_count: 0,
            setup: Setup::None,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.setup = Setup::None;
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    fn recent_extreme(&self, buf: &VecDeque<Decimal>, count: usize, want_max: bool) -> Decimal {
        buf.iter().take(count).copied().fold(
            if want_max { Decimal::MIN } else { Decimal::MAX },
            if want_max { Decimal::max } else { Decimal::min },
        )
    }

    /// 结构 A：回调接续。价格回落到 SMA50 附近后顺势收一根。
    fn signal_pullback(&self, bar: &Bar) -> Option<Signal> {
        if bar.sma50 > bar.sma200 {
            let near = bar.low <= bar.sma50 * A_BAND_UP && bar.low >= bar.sma50 * A_BAND_DOWN;
            let recent_low = self.recent_extreme(&self.low_buf, A_SWING, false);

            if near && bar.close > bar.open && bar.rsi > A_RSI_LONG.0 && bar.rsi < A_RSI_LONG.1 {
                // 结构止损与最小止损距离取更宽的那个
                let sl = round_to_tick(
                    recent_low
                        .min(bar.low)
                        .min(bar.close - A_SL_MIN_ATR * bar.atr),
                );
                let risk = bar.close - sl;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close + risk * A_TP_R);

                    if tp > bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        if bar.sma50 < bar.sma200 {
            let near = bar.high >= bar.sma50 * A_BAND_DOWN && bar.high <= bar.sma50 * A_BAND_UP;
            let recent_high = self.recent_extreme(&self.high_buf, A_SWING, true);

            if near && bar.close < bar.open && bar.rsi > A_RSI_SHORT.0 && bar.rsi < A_RSI_SHORT.1 {
                let sl = round_to_tick(
                    recent_high
                        .max(bar.high)
                        .max(bar.close + A_SL_MIN_ATR * bar.atr),
                );
                let risk = sl - bar.close;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close - risk * A_TP_R);

                    if tp < bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        None
    }

    /// 结构 B：动量延续。三线排列 + ADX 确认后，收盘突破前一根高点跟进。
    fn signal_continuation(&self, bar: &Bar) -> Option<Signal> {
        let trend_long =
            bar.close > bar.ema_slow && bar.ema_fast > bar.ema_mid && bar.ema_mid > bar.ema_slow;
        let trend_short =
            bar.close < bar.ema_slow && bar.ema_fast < bar.ema_mid && bar.ema_mid < bar.ema_slow;

        if trend_long
            && bar.adx >= B_ADX_LONG
            && bar.rsi >= B_RSI_LONG.0
            && bar.rsi <= B_RSI_LONG.1
            && bar.close > bar.prev_high
            && bar.close > bar.open
            && bar.close - bar.ema_fast <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_low = self.recent_extreme(&self.low_buf, B_SWING, false);
            let sl = round_to_tick(
                (swing_low - B_SL_BUFFER_ATR * bar.atr).min(bar.close - B_SL_ATR * bar.atr),
            );
            let risk = bar.close - sl;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close + B_TP_R * risk);

                if tp > bar.close {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        if trend_short
            && bar.adx >= B_ADX_SHORT
            && bar.rsi >= B_RSI_SHORT.0
            && bar.rsi <= B_RSI_SHORT.1
            && bar.close < bar.prev_low
            && bar.close < bar.open
            && bar.ema_fast - bar.close <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_high = self.recent_extreme(&self.high_buf, B_SWING, true);
            let sl = round_to_tick(
                (swing_high + B_SL_BUFFER_ATR * bar.atr).max(bar.close + B_SL_ATR * bar.atr),
            );
            let risk = sl - bar.close;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close - B_TP_R * risk);

                if tp < bar.close && tp > Decimal::ZERO {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        None
    }
}

#[async_trait(?Send)]
impl Strategy for DualStructureTrend {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);

        let keep = B_SWING.max(A_SWING);

        if self.high_buf.len() > keep {
            self.high_buf.truncate(keep);
        }

        if self.low_buf.len() > keep {
            self.low_buf.truncate(keep);
        }

        self.bar_count += 1;

        if self.bar_count < WARMUP {
            return Ok(());
        }

        let (Some(ema_fast), Some(ema_mid), Some(ema_slow)) = (ema_fast, ema_mid, ema_slow) else {
            return Ok(());
        };
        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };
        let Some(rsi) = self.rsi.value() else {
            return Ok(());
        };
        let Some(sma50) = sma(cx.close, 50) else {
            return Ok(());
        };
        let Some(sma200) = sma(cx.close, 200) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // ---------------- 持仓管理 ----------------
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            // 只有结构 B 使用移动止损；结构 A 固定 2R 止盈 + 结构止损
            if self.setup == Setup::Continuation {
                let long = position.side == Side::Buy;
                let sign = if long { Decimal::ONE } else { -Decimal::ONE };
                let favorable = (close - entry_price) * sign;

                if favorable > self.best_favorable {
                    self.best_favorable = favorable;
                }

                // 浮盈达到 B_TRAIL_START_R 后，止损跟随“最佳浮盈 - min(1 ATR, 0.5R)”
                if self.best_favorable >= risk * B_TRAIL_START_R {
                    let give_back = (B_TRAIL_GIVE_ATR * atr).min(B_TRAIL_GIVE_R * risk);
                    let candidate = entry_price + sign * (self.best_favorable - give_back);
                    let current_stop = self.stop_price.unwrap_or(Decimal::ZERO);
                    let improves = current_stop.is_zero()
                        || if long {
                            candidate > current_stop
                        } else {
                            candidate < current_stop
                        };

                    if improves {
                        let new_stop = round_to_tick(candidate);

                        if new_stop != current_stop {
                            if let Some(id) = self.stop_order_id.clone() {
                                _ = cx.cancel_order(SYMBOL, id).await;
                            }

                            self.stop_order_id = if long {
                                cx.sell_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            } else {
                                cx.buy_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            };
                            self.stop_price = Some(new_stop);
                        }
                    }
                }

                if self.bar_held >= B_MAX_BAR_HELD {
                    cx.cancel_all_order(SYMBOL).await?;
                    cx.close_all_position(SYMBOL).await?;
                    self.reset_trade();
                }
            }

            return Ok(());
        }

        self.reset_trade();

        let bar = Bar {
            open,
            high,
            low,
            close,
            prev_high: cx.high[1],
            prev_low: cx.low[1],
            atr,
            rsi,
            sma50,
            sma200,
            ema_fast,
            ema_mid,
            ema_slow,
            adx: adx.unwrap_or(Decimal::ZERO),
        };

        // 同一根 K 线两套都成立时优先用 B（PF 更高），否则先看 A
        for setup in [Setup::Continuation, Setup::Pullback] {
            let signal = match setup {
                Setup::Continuation => self.signal_continuation(&bar),
                _ => self.signal_pullback(&bar),
            };

            let Some(signal) = signal else {
                continue;
            };

            cx.cancel_all_order(SYMBOL).await?;

            let sl_result = if signal.tp > close {
                cx.buy_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            } else {
                cx.sell_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            };

            self.setup = setup;
            self.entry_price = Some(close);
            self.risk = Some(signal.risk);
            self.stop_price = Some(signal.sl);
            self.stop_order_id = sl_result.ok();

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}  profit/mdd={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
    );
    println!(
        "long/short = {}/{}",
        long_count,
        summary.total_trades - long_count
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, DualStructureTrend::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 4. `strategy_volume_breakout`

Donchian channel breakout confirmed by a volume spike。

```rust
use std::collections::VecDeque;
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

fn donchian(high: &[Decimal], low: &[Decimal], period: usize) -> (Decimal, Decimal) {
    let upper = high
        .iter()
        .take(period)
        .copied()
        .fold(Decimal::MIN, Decimal::max);
    let lower = low
        .iter()
        .take(period)
        .copied()
        .fold(Decimal::MAX, Decimal::min);
    (upper, lower)
}

fn vol_ratio(volume: &[Decimal], period: usize) -> Option<Decimal> {
    if volume.len() < period + 1 {
        return None;
    }
    let avg_vol: Decimal =
        volume.iter().skip(1).take(period).sum::<Decimal>() / Decimal::from(period);
    if avg_vol.is_zero() {
        None
    } else {
        Some(volume[0] / avg_vol)
    }
}

struct Rma {
    period: usize,
    buf: VecDeque<Decimal>,
    value: Option<Decimal>,
}

impl Rma {
    fn new(period: usize) -> Self {
        Rma {
            period,
            buf: VecDeque::new(),
            value: None,
        }
    }
    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        self.buf.push_front(price);
        if self.buf.len() < self.period {
            let sum: Decimal = self.buf.iter().sum();
            self.value = Some(sum / Decimal::from(self.buf.len()));
            return self.value;
        }
        if self.buf.len() == self.period {
            let sum: Decimal = self.buf.iter().sum();
            self.value = Some(sum / Decimal::from(self.period));
            return self.value;
        }
        let alpha = dec!(1) / Decimal::from(self.period);
        if let Some(prev) = self.value {
            self.value = Some(alpha * price + (dec!(1) - alpha) * prev);
        }
        self.value
    }
}

struct VolumeBreakout {
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    vol_buf: VecDeque<Decimal>,
    rma_close: Rma,
    rma_vol: Rma,
}

impl VolumeBreakout {
    fn new() -> Self {
        VolumeBreakout {
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            vol_buf: VecDeque::new(),
            rma_close: Rma::new(20),
            rma_vol: Rma::new(20),
        }
    }
}

#[async_trait(?Send)]
impl Strategy for VolumeBreakout {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let high = cx.high[0];
        let low = cx.low[0];
        let vol = cx.volume[0];

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);
        self.vol_buf.push_front(vol);

        if self.high_buf.len() < 25 {
            self.rma_close.update(cx.close[0]);
            self.rma_vol.update(vol);
            return Ok(());
        }

        let ma_price = self.rma_close.update(cx.close[0]);
        let _ma_vol = self.rma_vol.update(vol);
        let Some(ma_price) = ma_price else {
            return Ok(());
        };

        let h: Vec<Decimal> = self.high_buf.iter().copied().collect();
        let l: Vec<Decimal> = self.low_buf.iter().copied().collect();
        let v: Vec<Decimal> = self.vol_buf.iter().copied().collect();

        let (dc_u, dc_l) = donchian(&h, &l, 20);
        let vr = vol_ratio(&v, 20);

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        let atr_val = atr(cx.high, cx.low, cx.close, 14);
        let Some(atr) = atr_val else { return Ok(()) };

        // 多头：唐奇安上轨突破 + 放量 1.3x + 价格在均线上方
        let has_vol = vr.map_or(false, |r| r > dec!(1.3));
        if cx.high[0] >= dc_u && has_vol && cx.close[0] > ma_price {
            let sl = round_to_tick(ma_price - atr * dec!(0.5));
            let tp = round_to_tick(cx.close[0] + atr * dec!(4));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
            return Ok(());
        }

        // 空头：唐奇安下轨跌破 + 放量 1.3x + 价格在均线下方
        if cx.low[0] <= dc_l && has_vol && cx.close[0] < ma_price {
            let sl = round_to_tick(ma_price + atr * dec!(0.5));
            let tp = round_to_tick(cx.close[0] - atr * dec!(4));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        }

        Ok(())
    }
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, VolumeBreakout::new(), Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 5. `strategy_pullback_plus_breakout`

Pullback continuation plus momentum continuation sharing one risk budget。

```rust
//! 双结构趋势策略：回调接续 + 动量延续，两套入场共用一份风控。
//!
//! 设计动机：单一入场逻辑在 BTC 4h 上很难同时在短窗口和长窗口里都占优。
//! 实测（`strategy_pullback_trend.rs` 为基准）：
//!
//! - 回调类入场（原版思路）胜率约 40%、盈亏比约 1.9，靠“多打”取胜；
//! - 突破延续类入场胜率约 56%、PF 明显更高，但一年只有 30 笔左右，绝对利润不够。
//!
//! 两者信号不相关（一个在回落时买、一个在破新高时买），但都不能同时持仓，
//! 所以把两套入场放进同一个仓位槽、共用同一份仓位管理与风控，
//! 得到的是“交易数接近两者之和、PF 高于两者加权”的组合效果——这是分散化，
//! 不是把两个策略各自跑一遍。
//!
//! 结构：
//!
//! | | 结构 A：回调接续 | 结构 B：动量延续 |
//! |---|---|---|
//! | 趋势判定 | SMA50 vs SMA200 | EMA21 > EMA50 > EMA200 且站上 EMA200 |
//! | 强度过滤 | 无 | ADX(14) >= 24（做空 >= 30） |
//! | 入场触发 | 价格触及 SMA50 带宽 + 顺向 K 线 | 收盘突破前一根高/低点 |
//! | RSI | 40~65（空 35~60） | 38~68（空 32~62） |
//! | 不追高 | — | 距 EMA21 不超过 2 ATR |
//! | 止损 | 近 5 根极值 | 结构极值与 1.2 ATR 取宽者，风险限制 0.25~2.4 ATR |
//! | 止盈 | 2R 限价 | 4R 限价 |
//! | 移动止损 | 无 | 盈利 0.6R 后按 1 ATR 跟随 |
//! | 超时离场 | 无 | 60 根 |
//!
//! 同一根 K 线两套都成立时，优先用 B（PF 更高的那套）。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;
/// 指标预热：EMA200 之外再留 12 根给 ADX 收敛。
const WARMUP: usize = 212;

// ---- 结构 A ----
const A_RSI_LONG: (Decimal, Decimal) = (dec!(40), dec!(65));
const A_RSI_SHORT: (Decimal, Decimal) = (dec!(35), dec!(60));
const A_BAND_UP: Decimal = dec!(1.005);
const A_BAND_DOWN: Decimal = dec!(0.99);
const A_SWING: usize = 5;
const A_TP_R: Decimal = dec!(2.0);

// ---- 结构 B ----
const B_ADX_LONG: Decimal = dec!(24);
const B_ADX_SHORT: Decimal = dec!(30);
const B_RSI_LONG: (Decimal, Decimal) = (dec!(38), dec!(68));
const B_RSI_SHORT: (Decimal, Decimal) = (dec!(32), dec!(62));
const B_SWING: usize = 6;
const B_SL_ATR: Decimal = dec!(1.2);
const B_SL_BUFFER_ATR: Decimal = dec!(0.25);
const B_RISK_MIN_ATR: Decimal = dec!(0.25);
const B_RISK_MAX_ATR: Decimal = dec!(2.4);
const B_MAX_EXT_ATR: Decimal = dec!(2.0);
const B_TP_R: Decimal = dec!(4.0);
const B_TRAIL_START_R: Decimal = dec!(0.6);
const B_TRAIL_ATR: Decimal = dec!(1.0);
const B_MAX_BAR_HELD: usize = 60;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_w: Option<Decimal>,
    plus_w: Option<Decimal>,
    minus_w: Option<Decimal>,
    dx_sum: Decimal,
    dx_count: usize,
    value: Option<Decimal>,
}

impl Adx {
    fn new(period: usize) -> Self {
        Adx {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            tr_sum: Decimal::ZERO,
            plus_sum: Decimal::ZERO,
            minus_sum: Decimal::ZERO,
            bar_count: 0,
            tr_w: None,
            plus_w: None,
            minus_w: None,
            dx_sum: Decimal::ZERO,
            dx_count: 0,
            value: None,
        }
    }

    fn update(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let Some(prev_high) = self.prev_high else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            return None;
        };

        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
        self.bar_count += 1;

        let period = Decimal::from(self.period);

        match self.tr_w {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_w = Some(self.tr_sum);
                self.plus_w = Some(self.plus_sum);
                self.minus_w = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_w.unwrap();
                let minus = self.minus_w.unwrap();

                self.tr_w = Some(value - value / period + tr);
                self.plus_w = Some(plus - plus / period + plus_dm);
                self.minus_w = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_w = self.tr_w.unwrap();

        if tr_w.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_w.unwrap() / tr_w * dec!(100);
        let minus_di = self.minus_w.unwrap() / tr_w * dec!(100);
        let di_sum = plus_di + minus_di;

        if di_sum.is_zero() {
            return self.value;
        }

        let dx = (plus_di - minus_di).abs() / di_sum * dec!(100);

        match self.value {
            None => {
                self.dx_sum += dx;
                self.dx_count += 1;

                if self.dx_count >= self.period {
                    self.value = Some(self.dx_sum / period);
                }
            }
            Some(prev) => {
                self.value = Some((prev * (period - dec!(1)) + dx) / period);
            }
        }

        self.value
    }
}

/// 当前 K 线及其指标快照，供两套入场逻辑共用。
struct Bar {
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    prev_high: Decimal,
    prev_low: Decimal,
    atr: Decimal,
    rsi: Decimal,
    sma50: Decimal,
    sma200: Decimal,
    ema_fast: Decimal,
    ema_mid: Decimal,
    ema_slow: Decimal,
    adx: Decimal,
}

/// 一套入场结构给出的结果：(止盈价, 止损价, 单笔风险)。
struct Signal {
    tp: Decimal,
    sl: Decimal,
    risk: Decimal,
}

/// 持仓来源，决定用哪套离场规则。
#[derive(Clone, Copy, PartialEq)]
enum Setup {
    None,
    Pullback,
    Continuation,
}

struct DualTrend {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    bar_count: usize,
    setup: Setup,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl DualTrend {
    fn new() -> Self {
        DualTrend {
            ema_fast: EMACache::new(21),
            ema_mid: EMACache::new(50),
            ema_slow: EMACache::new(200),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            bar_count: 0,
            setup: Setup::None,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.setup = Setup::None;
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    fn recent_extreme(&self, buf: &VecDeque<Decimal>, count: usize, want_max: bool) -> Decimal {
        buf.iter().take(count).copied().fold(
            if want_max { Decimal::MIN } else { Decimal::MAX },
            if want_max { Decimal::max } else { Decimal::min },
        )
    }

    /// 结构 A：回调接续（原版思路，作为稳健底仓）。价格回落到 SMA50 附近后顺势收一根。
    fn signal_pullback(&self, bar: &Bar) -> Option<Signal> {
        if bar.sma50 > bar.sma200 {
            let near = bar.low <= bar.sma50 * A_BAND_UP && bar.low >= bar.sma50 * A_BAND_DOWN;
            let recent_low = self.recent_extreme(&self.low_buf, A_SWING, false);

            if near && bar.close > bar.open && bar.rsi > A_RSI_LONG.0 && bar.rsi < A_RSI_LONG.1 {
                let sl = round_to_tick(recent_low.min(bar.low));
                let risk = bar.close - sl;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close + risk * A_TP_R);

                    if tp > bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        if bar.sma50 < bar.sma200 {
            let near = bar.high >= bar.sma50 * A_BAND_DOWN && bar.high <= bar.sma50 * A_BAND_UP;
            let recent_high = self.recent_extreme(&self.high_buf, A_SWING, true);

            if near && bar.close < bar.open && bar.rsi > A_RSI_SHORT.0 && bar.rsi < A_RSI_SHORT.1 {
                let sl = round_to_tick(recent_high.max(bar.high));
                let risk = sl - bar.close;

                if risk > Decimal::ZERO {
                    let tp = round_to_tick(bar.close - risk * A_TP_R);

                    if tp < bar.close {
                        return Some(Signal { tp, sl, risk });
                    }
                }
            }
        }

        None
    }

    /// 结构 B：动量延续。三线排列 + ADX 确认后，收盘突破前一根高点跟进，用 ATR 移动止损持有。
    fn signal_continuation(&self, bar: &Bar) -> Option<Signal> {
        let trend_long =
            bar.close > bar.ema_slow && bar.ema_fast > bar.ema_mid && bar.ema_mid > bar.ema_slow;
        let trend_short =
            bar.close < bar.ema_slow && bar.ema_fast < bar.ema_mid && bar.ema_mid < bar.ema_slow;

        if trend_long
            && bar.adx >= B_ADX_LONG
            && bar.rsi >= B_RSI_LONG.0
            && bar.rsi <= B_RSI_LONG.1
            && bar.close > bar.prev_high
            && bar.close > bar.open
            && bar.close - bar.ema_fast <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_low = self.recent_extreme(&self.low_buf, B_SWING, false);
            let sl = round_to_tick(
                (swing_low - B_SL_BUFFER_ATR * bar.atr).min(bar.close - B_SL_ATR * bar.atr),
            );
            let risk = bar.close - sl;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close + B_TP_R * risk);

                if tp > bar.close {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        if trend_short
            && bar.adx >= B_ADX_SHORT
            && bar.rsi >= B_RSI_SHORT.0
            && bar.rsi <= B_RSI_SHORT.1
            && bar.close < bar.prev_low
            && bar.close < bar.open
            && bar.ema_fast - bar.close <= B_MAX_EXT_ATR * bar.atr
        {
            let swing_high = self.recent_extreme(&self.high_buf, B_SWING, true);
            let sl = round_to_tick(
                (swing_high + B_SL_BUFFER_ATR * bar.atr).max(bar.close + B_SL_ATR * bar.atr),
            );
            let risk = sl - bar.close;

            if risk >= B_RISK_MIN_ATR * bar.atr && risk <= B_RISK_MAX_ATR * bar.atr {
                let tp = round_to_tick(bar.close - B_TP_R * risk);

                if tp < bar.close && tp > Decimal::ZERO {
                    return Some(Signal { tp, sl, risk });
                }
            }
        }

        None
    }
}

#[async_trait(?Send)]
impl Strategy for DualTrend {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);

        let keep = B_SWING.max(A_SWING);

        if self.high_buf.len() > keep {
            self.high_buf.truncate(keep);
        }

        if self.low_buf.len() > keep {
            self.low_buf.truncate(keep);
        }

        self.bar_count += 1;

        if self.bar_count < WARMUP {
            return Ok(());
        }

        let (Some(ema_fast), Some(ema_mid), Some(ema_slow)) = (ema_fast, ema_mid, ema_slow) else {
            return Ok(());
        };
        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };
        let Some(rsi) = self.rsi.value() else {
            return Ok(());
        };
        let Some(sma50) = sma(cx.close, 50) else {
            return Ok(());
        };
        let Some(sma200) = sma(cx.close, 200) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // ---------------- 持仓管理 ----------------
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            // 只有结构 B 使用移动止损；结构 A 固定 2R 止盈 + 结构止损
            if self.setup == Setup::Continuation {
                let long = position.side == Side::Buy;
                let sign = if long { Decimal::ONE } else { -Decimal::ONE };
                let favorable = (close - entry_price) * sign;

                if favorable > self.best_favorable {
                    self.best_favorable = favorable;
                }

                if self.best_favorable >= risk * B_TRAIL_START_R {
                    let candidate = entry_price + sign * (self.best_favorable - B_TRAIL_ATR * atr);
                    let current_stop = self.stop_price.unwrap_or(Decimal::ZERO);
                    let improves = current_stop.is_zero()
                        || if long {
                            candidate > current_stop
                        } else {
                            candidate < current_stop
                        };

                    if improves {
                        let new_stop = round_to_tick(candidate);

                        if new_stop != current_stop {
                            if let Some(id) = self.stop_order_id.clone() {
                                _ = cx.cancel_order(SYMBOL, id).await;
                            }

                            self.stop_order_id = if long {
                                cx.sell_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            } else {
                                cx.buy_trigger_market_reduce_only(
                                    SYMBOL,
                                    new_stop,
                                    position.quantity,
                                )
                                .await
                                .ok()
                            };
                            self.stop_price = Some(new_stop);
                        }
                    }
                }

                if self.bar_held >= B_MAX_BAR_HELD {
                    cx.cancel_all_order(SYMBOL).await?;
                    cx.close_all_position(SYMBOL).await?;
                    self.reset_trade();
                }
            }

            return Ok(());
        }

        self.reset_trade();

        let bar = Bar {
            open,
            high,
            low,
            close,
            prev_high: cx.high[1],
            prev_low: cx.low[1],
            atr,
            rsi,
            sma50,
            sma200,
            ema_fast,
            ema_mid,
            ema_slow,
            adx: adx.unwrap_or(Decimal::ZERO),
        };

        // 同一根 K 线两套都成立时优先用 B（PF 更高），否则先看 A
        for setup in [Setup::Continuation, Setup::Pullback] {
            let signal = match setup {
                Setup::Continuation => self.signal_continuation(&bar),
                _ => self.signal_pullback(&bar),
            };

            let Some(signal) = signal else {
                continue;
            };

            cx.cancel_all_order(SYMBOL).await?;

            let sl_result = if signal.tp > close {
                cx.buy_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            } else {
                cx.sell_tp_sl(SYMBOL, signal.tp, signal.sl, QTY).await?.2
            };

            self.setup = setup;
            self.entry_price = Some(close);
            self.risk = Some(signal.risk);
            self.stop_price = Some(signal.sl);
            self.stop_order_id = sl_result.ok();

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}  profit/mdd={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
    );
    println!(
        "long/short = {}/{}",
        long_count,
        summary.total_trades - long_count
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, DualTrend::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 6. `strategy_rsi_ema`

Consecutive-candle breakout filtered by RSI/EMA。

```rust
use trading_maid::prelude::*;

fn round_to_tick(price: rust_decimal::Decimal) -> rust_decimal::Decimal {
    let tick = rust_decimal_macros::dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= rust_decimal::Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if cx.close.len() < 30 {
        return Ok(());
    }

    if cx.get_position("BTCUSDT").await?.is_some() {
        return Ok(());
    }

    let sma50 = sma(cx.close, 50);
    let atr_val = atr(cx.high, cx.low, cx.close, 14);
    let (Some(sma50), Some(atr)) = (sma50, atr_val) else {
        return Ok(());
    };

    let body0 = (cx.close[0] - cx.open[0]).abs();

    let vol_ma: rust_decimal::Decimal = (1..=10)
        .filter_map(|i| cx.volume.get(i))
        .sum::<rust_decimal::Decimal>()
        / rust_decimal_macros::dec!(10);
    let vol_ok = vol_ma > rust_decimal::Decimal::ZERO && cx.volume[0] > vol_ma;

    if cx.close[0] > cx.open[0]
        && cx.close[1] > cx.open[1]
        && body0 >= atr * rust_decimal_macros::dec!(0.4)
        && cx.close[0] > cx.high[1]
        && cx.close[0] > sma50
        && vol_ok
    {
        let low_2 = cx.low[1].min(cx.low[0]);
        let sl = round_to_tick(low_2);
        let tp = round_to_tick(cx.close[0] + atr * rust_decimal_macros::dec!(3.9));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        return Ok(());
    }

    if cx.close[0] < cx.open[0]
        && cx.close[1] < cx.open[1]
        && body0 >= atr * rust_decimal_macros::dec!(0.4)
        && cx.close[0] < cx.low[1]
        && cx.close[0] < sma50
        && vol_ok
    {
        let high_2 = cx.high[1].max(cx.high[0]);
        let sl = round_to_tick(high_2);
        let tp = round_to_tick(cx.close[0] - atr * rust_decimal_macros::dec!(3.9));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
    }

    Ok(())
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, my_strategy, Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 7. `strategy_price_action`

Momentum breakout, indicators written from scratch。

```rust
use std::collections::VecDeque;
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

// 动量评分：最近 N 根 K 线的上涨力度总和
fn momentum_score(close: &[Decimal], n: usize) -> Decimal {
    if close.len() < n + 1 {
        return dec!(0);
    }
    let mut score = dec!(0);
    for i in 0..n {
        let change = (close[i] - close[i + 1]) / close[i + 1] * dec!(100);
        score = score + change;
    }
    score
}

// 检测成交量爆发：当前成交量 vs 之前 N 根均值
fn volume_spike(volume: &[Decimal], n: usize) -> Option<Decimal> {
    if volume.len() < n + 1 {
        return None;
    }
    let avg: Decimal = volume.iter().skip(1).take(n).sum::<Decimal>() / Decimal::from(n);
    if avg.is_zero() {
        return None;
    }
    Some(volume[0] / avg)
}

// 平均真实波幅（简化版）
fn avg_range(high: &[Decimal], low: &[Decimal], n: usize) -> Decimal {
    if high.len() < n || low.len() < n {
        return dec!(300);
    }
    let sum: Decimal = high
        .iter()
        .zip(low.iter())
        .take(n)
        .map(|(h, l)| h - l)
        .sum();
    sum / Decimal::from(n)
}

struct Momentum {
    close_buf: VecDeque<Decimal>,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    vol_buf: VecDeque<Decimal>,
}

impl Momentum {
    fn new() -> Self {
        Momentum {
            close_buf: VecDeque::new(),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            vol_buf: VecDeque::new(),
        }
    }
}

#[async_trait(?Send)]
impl Strategy for Momentum {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.close_buf.push_front(cx.close[0]);
        self.high_buf.push_front(cx.high[0]);
        self.low_buf.push_front(cx.low[0]);
        self.vol_buf.push_front(cx.volume[0]);

        if self.high_buf.len() < 8 {
            return Ok(());
        }

        let c: Vec<Decimal> = self.close_buf.iter().copied().collect();
        let h: Vec<Decimal> = self.high_buf.iter().copied().collect();
        let l: Vec<Decimal> = self.low_buf.iter().copied().collect();
        let v: Vec<Decimal> = self.vol_buf.iter().copied().collect();

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        let score = momentum_score(&c, 3);
        let spike = volume_spike(&v, 5);
        let range = avg_range(&h, &l, 5);
        let body = (cx.open[0] - cx.close[0]).abs();

        // 多头：连续 3 根累积极动量 > 2% + 成交量爆发 1.5x + 实体大
        if score > dec!(2)
            && body >= range
            && body >= dec!(200)
            && spike.map_or(false, |s| s > dec!(1.5))
        {
            let sl = round_to_tick(c[0] - range * dec!(1.5));
            let tp = round_to_tick(c[0] + range * dec!(3));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
            return Ok(());
        }

        // 空头：连续 3 根累积极动量 < -2% + 成交量爆发
        if score < dec!(-2)
            && body >= range
            && body >= dec!(200)
            && spike.map_or(false, |s| s > dec!(1.5))
        {
            let sl = round_to_tick(c[0] + range * dec!(1.5));
            let tp = round_to_tick(c[0] - range * dec!(3));
            cx.cancel_all_order("BTCUSDT").await?;
            _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        }

        Ok(())
    }
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, Momentum::new(), Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 8. `strategy_pullback_continuation`

EMA stack + ADX strength + break-of-prior-high entry + ATR trailing stop。

```rust
//! 趋势回调续势策略：EMA 排列定方向、ADX 定强度、突破前高定入场、ATR 移动止损定离场。
//!
//! 相对 `strategy_pullback_trend.rs` 的改进点：
//!
//! | 维度 | strategy_pullback_trend.rs | 本策略 |
//! |------|-------------------|--------|
//! | 趋势判定 | SMA50 vs SMA200 两条线 | EMA21 > EMA50 > EMA200 三线排列 + 收盘在 EMA200 之上 |
//! | 趋势强度 | 无 | Wilder ADX(14)：做多 >= 24，做空 >= 30 |
//! | 回调质量 | 与 SMA50 固定百分比带宽（0.99~1.005） | ATR 归一化，且不追高（收盘距 EMA21 <= 2 ATR） |
//! | 入场触发 | 阳线 + RSI 区间 | 收盘突破前一根高点（动量确认），避免下跌途中接刀 |
//! | 止损 | 最近 5 根极值（无风险约束） | 结构极值与 1.2 ATR 取宽者，且把单笔风险限制在 0.25~2.4 ATR |
//! | 止盈 | 固定 2R | 4R 限价止盈 + 1 ATR 移动止损（盈利 0.6R 后启动） |
//! | 出场 | 只能等 TP/SL | 另有持仓超时离场，移动止损只上移不下移 |
//!
//! 指标全部用带状态的缓存（`EMACache` / `RSICache` / 自实现 ADX），每根 K 线 O(1) 更新，
//! 不会像原版那样每根都重算整条序列。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const QTY: Decimal = dec!(0.01);
const TICK: Decimal = dec!(0.1);

/// ATR 与 ADX 的周期。
const ATR_N: usize = 14;
/// 结构极值回看的 K 线根数。
const SWING: usize = 6;
/// 三线 EMA 周期。
const EMA_FAST: usize = 21;
const EMA_MID: usize = 50;
const EMA_SLOW: usize = 200;

/// 做多/做空的趋势强度门槛。加密资产长期上行，做空需要更强的趋势证据。
const ADX_LONG: Decimal = dec!(24);
const ADX_SHORT: Decimal = dec!(30);
/// 入场时的 RSI 区间（做空取其镜像 32~62）。RSI 过低说明回调尚未结束。
const RSI_LOW: Decimal = dec!(38);
const RSI_HIGH: Decimal = dec!(68);

/// 收盘价距离快线的最大偏离，避免在已经走远的行情里追高。
const MAX_EXT_ATR: Decimal = dec!(2.0);
/// 止损的结构缓冲，以及止损距离的下限/上限（单位 ATR）。
const SL_BUFFER_ATR: Decimal = dec!(0.25);
const SL_MIN_ATR: Decimal = dec!(1.2);
const RISK_MIN_ATR: Decimal = dec!(0.25);
const RISK_MAX_ATR: Decimal = dec!(2.4);

/// 止盈 = 4R；盈利达到 0.6R 后启用 1 ATR 移动止损。
const TP_R: Decimal = dec!(4.0);
const TRAIL_START_R: Decimal = dec!(0.6);
const TRAIL_ATR: Decimal = dec!(1.0);
/// 持仓超过该根数仍未出场则市价离场，释放资金并避免长期占用。
const MAX_BAR_HELD: usize = 60;

/// 指标预热：EMA200 之外再留 12 根给 ADX 收敛。
const WARMUP: usize = EMA_SLOW + 12;

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// Wilder 的 ADX / 方向运动。
///
/// 先按周期累加得到 TR / +DM / -DM 的初始平滑值，之后每次
/// `value = value - value / period + 新值`；DX 再做一次 Wilder 平滑得到 ADX。
struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    tr_sum: Decimal,
    plus_sum: Decimal,
    minus_sum: Decimal,
    bar_count: usize,
    tr_wilder: Option<Decimal>,
    plus_wilder: Option<Decimal>,
    minus_wilder: Option<Decimal>,
    dx_sum: Decimal,
    dx_count: usize,
    value: Option<Decimal>,
}

impl Adx {
    fn new(period: usize) -> Self {
        Adx {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            tr_sum: Decimal::ZERO,
            plus_sum: Decimal::ZERO,
            minus_sum: Decimal::ZERO,
            bar_count: 0,
            tr_wilder: None,
            plus_wilder: None,
            minus_wilder: None,
            dx_sum: Decimal::ZERO,
            dx_count: 0,
            value: None,
        }
    }

    fn update(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let Some(prev_high) = self.prev_high else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);

            return None;
        };

        let prev_low = self.prev_low.unwrap();
        let prev_close = self.prev_close.unwrap();

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);
        self.bar_count += 1;

        let period = Decimal::from(self.period);

        match self.tr_wilder {
            None => {
                self.tr_sum += tr;
                self.plus_sum += plus_dm;
                self.minus_sum += minus_dm;

                if self.bar_count < self.period {
                    return None;
                }

                self.tr_wilder = Some(self.tr_sum);
                self.plus_wilder = Some(self.plus_sum);
                self.minus_wilder = Some(self.minus_sum);
            }
            Some(value) => {
                let plus = self.plus_wilder.unwrap();
                let minus = self.minus_wilder.unwrap();

                self.tr_wilder = Some(value - value / period + tr);
                self.plus_wilder = Some(plus - plus / period + plus_dm);
                self.minus_wilder = Some(minus - minus / period + minus_dm);
            }
        }

        let tr_wilder = self.tr_wilder.unwrap();

        if tr_wilder.is_zero() {
            return self.value;
        }

        let plus_di = self.plus_wilder.unwrap() / tr_wilder * dec!(100);
        let minus_di = self.minus_wilder.unwrap() / tr_wilder * dec!(100);
        let di_sum = plus_di + minus_di;

        if di_sum.is_zero() {
            return self.value;
        }

        let dx = (plus_di - minus_di).abs() / di_sum * dec!(100);

        match self.value {
            None => {
                self.dx_sum += dx;
                self.dx_count += 1;

                if self.dx_count >= self.period {
                    self.value = Some(self.dx_sum / period);
                }
            }
            Some(prev) => {
                self.value = Some((prev * (period - dec!(1)) + dx) / period);
            }
        }

        self.value
    }
}

struct PullbackContinuation {
    ema_fast: EMACache,
    ema_mid: EMACache,
    ema_slow: EMACache,
    rsi: RSICache,
    adx: Adx,
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
    bar_count: usize,
    entry_price: Option<Decimal>,
    risk: Option<Decimal>,
    bar_held: usize,
    stop_price: Option<Decimal>,
    stop_order_id: Option<String>,
    best_favorable: Decimal,
}

impl PullbackContinuation {
    fn new() -> Self {
        PullbackContinuation {
            ema_fast: EMACache::new(EMA_FAST),
            ema_mid: EMACache::new(EMA_MID),
            ema_slow: EMACache::new(EMA_SLOW),
            rsi: RSICache::new(14),
            adx: Adx::new(ATR_N),
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
            bar_count: 0,
            entry_price: None,
            risk: None,
            bar_held: 0,
            stop_price: None,
            stop_order_id: None,
            best_favorable: Decimal::ZERO,
        }
    }

    fn reset_trade(&mut self) {
        self.entry_price = None;
        self.risk = None;
        self.bar_held = 0;
        self.stop_price = None;
        self.stop_order_id = None;
        self.best_favorable = Decimal::ZERO;
    }

    /// 结构极值 + ATR 下限构成的止损价，`long` 为 false 时取镜像。
    fn stop_price(&self, long: bool, close: Decimal, atr: Decimal) -> Decimal {
        if long {
            let swing_low = self
                .low_buf
                .iter()
                .take(SWING)
                .copied()
                .fold(Decimal::MAX, Decimal::min);

            round_to_tick((swing_low - SL_BUFFER_ATR * atr).min(close - SL_MIN_ATR * atr))
        } else {
            let swing_high = self
                .high_buf
                .iter()
                .take(SWING)
                .copied()
                .fold(Decimal::MIN, Decimal::max);

            round_to_tick((swing_high + SL_BUFFER_ATR * atr).max(close + SL_MIN_ATR * atr))
        }
    }
}

#[async_trait(?Send)]
impl Strategy for PullbackContinuation {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        // ---- 指标状态每根都必须推进，预热期也不能漏 ----
        let ema_fast = self.ema_fast.update(close);
        let ema_mid = self.ema_mid.update(close);
        let ema_slow = self.ema_slow.update(close);
        let _ = self.rsi.update(close);
        let adx = self.adx.update(high, low, close);

        self.high_buf.push_front(high);
        self.low_buf.push_front(low);

        if self.high_buf.len() > SWING {
            self.high_buf.truncate(SWING);
        }

        if self.low_buf.len() > SWING {
            self.low_buf.truncate(SWING);
        }

        self.bar_count += 1;

        if self.bar_count < WARMUP {
            return Ok(());
        }

        let (Some(ema_fast), Some(ema_mid), Some(ema_slow)) = (ema_fast, ema_mid, ema_slow) else {
            return Ok(());
        };
        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };
        let Some(rsi) = self.rsi.value() else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        // ---- 持仓管理：移动止损 + 超时离场 ----
        if let Some(position) = cx.get_position(SYMBOL).await? {
            self.bar_held += 1;

            let (Some(entry_price), Some(risk)) = (self.entry_price, self.risk) else {
                return Ok(());
            };

            let long = position.side == Side::Buy;
            let sign = if long { Decimal::ONE } else { -Decimal::ONE };
            let favorable = (close - entry_price) * sign;

            if favorable > self.best_favorable {
                self.best_favorable = favorable;
            }

            // 盈利达到 TRAIL_START_R 后，止损跟随“最佳收盘价 ± TRAIL_ATR * ATR”，只朝有利方向移动
            if self.best_favorable >= risk * TRAIL_START_R {
                let candidate = entry_price + sign * (self.best_favorable - TRAIL_ATR * atr);
                let current_stop = self.stop_price.unwrap_or(Decimal::ZERO);
                let improves = current_stop.is_zero()
                    || if long {
                        candidate > current_stop
                    } else {
                        candidate < current_stop
                    };

                if improves {
                    let new_stop = round_to_tick(candidate);

                    if new_stop != current_stop {
                        if let Some(id) = self.stop_order_id.clone() {
                            _ = cx.cancel_order(SYMBOL, id).await;
                        }

                        self.stop_order_id = if long {
                            cx.sell_trigger_market_reduce_only(SYMBOL, new_stop, position.quantity)
                                .await
                                .ok()
                        } else {
                            cx.buy_trigger_market_reduce_only(SYMBOL, new_stop, position.quantity)
                                .await
                                .ok()
                        };
                        self.stop_price = Some(new_stop);
                    }
                }
            }

            if self.bar_held >= MAX_BAR_HELD {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.reset_trade();
            }

            return Ok(());
        }

        self.reset_trade();

        // ---- 入场条件 ----
        let adx_value = adx.unwrap_or(Decimal::ZERO);
        let rsi_long_ok = rsi >= RSI_LOW && rsi <= RSI_HIGH;
        let rsi_short_ok =
            rsi >= Decimal::from(100) - RSI_HIGH && rsi <= Decimal::from(100) - RSI_LOW;

        let trend_long = close > ema_slow && ema_fast > ema_mid && ema_mid > ema_slow;
        let trend_short = close < ema_slow && ema_fast < ema_mid && ema_mid < ema_slow;
        let breakout_long = close > cx.high[1] && close > open;
        let breakout_short = close < cx.low[1] && close < open;

        if trend_long
            && adx_value >= ADX_LONG
            && rsi_long_ok
            && breakout_long
            && close - ema_fast <= MAX_EXT_ATR * atr
        {
            let sl = self.stop_price(true, close, atr);
            let risk = close - sl;

            if risk >= RISK_MIN_ATR * atr && risk <= RISK_MAX_ATR * atr {
                let tp = round_to_tick(close + TP_R * risk);

                if tp > close {
                    cx.cancel_all_order(SYMBOL).await?;
                    let (_, _, sl_result) = cx.buy_tp_sl(SYMBOL, tp, sl, QTY).await?;
                    self.entry_price = Some(close);
                    self.risk = Some(risk);
                    self.stop_price = Some(sl);
                    self.stop_order_id = sl_result.ok();

                    return Ok(());
                }
            }
        }

        if trend_short
            && adx_value >= ADX_SHORT
            && rsi_short_ok
            && breakout_short
            && ema_fast - close <= MAX_EXT_ATR * atr
        {
            let sl = self.stop_price(false, close, atr);
            let risk = sl - close;

            if risk >= RISK_MIN_ATR * atr && risk <= RISK_MAX_ATR * atr {
                let tp = round_to_tick(close - TP_R * risk);

                if tp < close && tp > Decimal::ZERO {
                    cx.cancel_all_order(SYMBOL).await?;
                    let (_, _, sl_result) = cx.sell_tp_sl(SYMBOL, tp, sl, QTY).await?;
                    self.entry_price = Some(close);
                    self.risk = Some(risk);
                    self.stop_price = Some(sl);
                    self.stop_order_id = sl_result.ok();
                }
            }
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}

#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, PullbackContinuation::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 9. `strategy_fib_martingale`

Fibonacci retracement + liquidity sweep + vacuum zone + add-on entries (0.382)。

```rust
//! 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（合约马丁格尔）
//!
//! 规则完全按设计说明实现，参数不做扫描：
//!
//! 1. **拐点流动性 / 双顶双底**：用 3-3 分形找已确认的拐点。要求最近两个拐点高点
//!    （或低点）价格相近（≤ 1 ATR），构成双顶/双底——这些位置堆积流动性。
//! 2. **斐波那契坐标**：`0 = 顶`，`1 = 底`（做多时反过来 `0 = 底`，`1 = 顶`）。
//!    - 入场：价格回撤到 `0.382`
//!    - 止损基准：`0`（前高/前低），但按解套理论不用它硬止损
//!    - 止盈：`1.382`
//! 3. **真空区/缺口**：要求这段摆动里至少有一根实体 ≥ 1 ATR 的急促 K 线。
//!    这种价位没有像样的止损盘，反向砸起来也快，所以延续概率更高。
//! 4. **解套理论 + 马丁格尔**：入场后价格不但没回来，反而突破前高（做空方向即 `0` 位），
//!    按 `前高 + 0.1R` 加 0.01、`前高 + 0.2R` 再加 0.02，最多加两次（总 0.04）。
//!    加仓把均价拉到前高附近，等价格回抽前高时**保本/解套离场**；
//!    第三次加仓之后若继续逆行，用 `前高 + 1 ATR` 的硬止损兜底。
//!
//! 注意：马丁格尔让单笔最大仓位到 0.04（基础单的 4 倍），
//! 所以绝对利润不能和前面几个固定 0.01 手位的策略直接横向比较。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;

/// 基础单与两次加仓的仓位
const BASE_QTY: Decimal = dec!(0.01);
const ADD1_QTY: Decimal = dec!(0.01);
const ADD2_QTY: Decimal = dec!(0.02);

/// 分形拐点的左右宽度
const PIVOT_SPAN: usize = 3;
/// 双顶/双底的价格相近度（ATR 倍数）
const DOUBLE_TOL_ATR: Decimal = dec!(1.0);
/// 摆动最小幅度（ATR 倍数），太小的摆动没有交易价值
const MIN_RANGE_ATR: Decimal = dec!(2.0);
/// 真空区判定：摆动内最大实体至少这么多 ATR
const VACUUM_BODY_ATR: Decimal = dec!(1.0);
/// 回撤/止盈/加仓的斐波那契位
const ENTRY_LEVEL: Decimal = dec!(0.382);
const TAKE_LEVEL: Decimal = dec!(1.382);
const ADD1_LEVEL: Decimal = dec!(-0.1);
const ADD2_LEVEL: Decimal = dec!(-0.2);
/// 三次加仓后的硬止损（ATR 倍数，位于 0 位之外）
const DISASTER_STOP_ATR: Decimal = dec!(1.0);

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// 一根 K 线的快照
#[derive(Clone)]
struct Candle {
    high: Decimal,
    low: Decimal,
    open: Decimal,
    close: Decimal,
}

impl Candle {
    fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }
}

/// 一个正在跟踪的斐波那契摆动
#[derive(Clone, Copy)]
struct Swing {
    /// 0 位价格（双顶的最高点 / 双底的最低点）
    origin: Decimal,
    /// 1 位价格（对面摆动极值）
    opposite: Decimal,
}

impl Swing {
    fn range(&self) -> Decimal {
        (self.origin - self.opposite).abs()
    }

    /// 按 0=顶、1=底 的坐标取价位；做多时 origin 是底，方向自动反过来。
    fn price_at(&self, level: Decimal) -> Decimal {
        self.origin + (self.opposite - self.origin) * level
    }
}

/// 一次持仓的马丁格尔阶梯
struct Ladder {
    long_side: bool,
    swing: Swing,
    unit_total: Decimal,
    add_count: usize,
    addition_list: [Decimal; 2],
    average_entry: Decimal,
}

struct FibMartingale {
    candle_list: VecDeque<Candle>,
    close_cache: VecDeque<Decimal>,
    bar_count: usize,
    ladder: Option<Ladder>,
    used_origin: Option<Decimal>,
}

impl FibMartingale {
    fn new() -> Self {
        FibMartingale {
            candle_list: VecDeque::new(),
            close_cache: VecDeque::new(),
            bar_count: 0,
            ladder: None,
            used_origin: None,
        }
    }

    /// 在 newest-first 的缓冲里找第 `skip` 个已确认拐点高点（返回索引）。
    fn find_pivot_high(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].high;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].high < value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].high < value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    fn find_pivot_low(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].low;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].low > value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].low > value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    /// 摆动区间内是否出现过急促实体（真空区）。
    fn has_vacuum(&self, from: usize, to: usize, atr: Decimal) -> bool {
        if from <= to {
            return false;
        }

        let threshold = VACUUM_BODY_ATR * atr;

        (to..from).any(|i| self.candle_list[i].body() >= threshold)
    }

    /// 找最近的“双顶”，返回摆动（0 = 顶，1 = 底）。
    fn find_double_top(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_high(0)?;
        let previous = self.find_pivot_high(1)?;
        let top = self.candle_list[last].high;
        let top_previous = self.candle_list[previous].high;

        if (top - top_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let bottom = (0..last)
            .map(|i| self.candle_list[i].low)
            .fold(Decimal::MAX, Decimal::min);
        let swing = Swing {
            origin: top,
            opposite: bottom,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }

    /// 找最近的“双底”，返回摆动（0 = 底，1 = 顶）。
    fn find_double_bottom(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_low(0)?;
        let previous = self.find_pivot_low(1)?;
        let bottom = self.candle_list[last].low;
        let bottom_previous = self.candle_list[previous].low;

        if (bottom - bottom_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let top = (0..last)
            .map(|i| self.candle_list[i].high)
            .fold(Decimal::MIN, Decimal::max);
        let swing = Swing {
            origin: bottom,
            opposite: top,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }
}

#[async_trait(?Send)]
impl Strategy for FibMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        self.candle_list.push_front(Candle {
            high,
            low,
            open,
            close,
        });

        if self.candle_list.len() > 80 {
            self.candle_list.truncate(80);
        }

        self.close_cache.push_front(close);

        if self.close_cache.len() > 200 {
            self.close_cache.truncate(200);
        }

        self.bar_count += 1;

        if self.bar_count < 60 {
            return Ok(());
        }

        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        let position = cx.get_position(SYMBOL).await?;

        // ---------------- 持仓管理：加仓 / 解套 / 硬止损 ----------------
        if position.is_some() {
            let Some(ladder) = self.ladder.as_mut() else {
                return Ok(());
            };

            let long_side = ladder.long_side;
            let origin = ladder.swing.origin;

            // 硬止损：0 位之外 1 ATR（由交易所的止损单兜底，这里只做状态清理）
            let disaster = if long_side {
                round_to_tick(origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(origin + DISASTER_STOP_ATR * atr)
            };
            let hit_disaster = if long_side {
                low <= disaster
            } else {
                high >= disaster
            };

            // 解套：已经加过仓，价格回抽 0 位（前高/前低）→ 保本离场
            let escape = ladder.add_count > 0
                && if long_side {
                    high >= origin
                } else {
                    low <= origin
                };

            // 加仓：价格突破 0 位一段距离
            let next_add = if ladder.add_count < ladder.addition_list.len() {
                Some(ladder.addition_list[ladder.add_count])
            } else {
                None
            };
            let need_add = next_add
                .map(|price| {
                    if long_side {
                        low <= price
                    } else {
                        high >= price
                    }
                })
                .unwrap_or(false);

            if need_add {
                let add_quantity = if ladder.add_count == 0 {
                    ADD1_QTY
                } else {
                    ADD2_QTY
                };
                let price = next_add.unwrap();

                // 用挂单价估算均价；实际成交价由撮合决定，方向一致即可
                let previous_notional = ladder.average_entry * ladder.unit_total;
                let new_total = ladder.unit_total + add_quantity;

                ladder.average_entry = (previous_notional + price * add_quantity) / new_total;
                ladder.unit_total = new_total;
                ladder.add_count += 1;

                let total = ladder.unit_total;
                let take = round_to_tick(ladder.swing.price_at(TAKE_LEVEL));

                // 必须先撤旧单，再下加仓单：cancel_all_order 会把刚挂出的市价单一起撤掉
                cx.cancel_all_order(SYMBOL).await?;

                if long_side {
                    _ = cx.buy(SYMBOL, add_quantity).await?;
                } else {
                    _ = cx.sell(SYMBOL, add_quantity).await?;
                }

                // 仓位变了，止盈止损要按新的总仓位重挂
                if long_side {
                    _ = cx.sell_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .sell_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                } else {
                    _ = cx.buy_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .buy_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                }

                return Ok(());
            }

            if hit_disaster || escape {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.ladder = None;
            }

            return Ok(());
        }

        self.ladder = None;

        // ---------------- 入场 ----------------
        let long_setup = self.find_double_bottom(atr);
        let short_setup = self.find_double_top(atr);

        // 先看空（双顶）再看多（双底）；同一根只做一笔
        for (long_side, swing) in [(false, short_setup), (true, long_setup)] {
            let Some(swing) = swing else {
                continue;
            };

            if self.used_origin == Some(swing.origin) {
                continue;
            }

            let entry_level = swing.price_at(ENTRY_LEVEL);
            let reached = if long_side {
                low <= entry_level
            } else {
                high >= entry_level
            };

            // 必须仍然在 0 位之内（还没被突破），否则这套结构已经失效
            let inside = if long_side {
                close > swing.origin
            } else {
                close < swing.origin
            };

            if !reached || !inside {
                continue;
            }

            let take = round_to_tick(swing.price_at(TAKE_LEVEL));
            let disaster = if long_side {
                round_to_tick(swing.origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(swing.origin + DISASTER_STOP_ATR * atr)
            };

            if (long_side && take <= close) || (!long_side && take >= close) {
                continue;
            }

            cx.cancel_all_order(SYMBOL).await?;

            if long_side {
                _ = cx.buy(SYMBOL, BASE_QTY).await?;
                _ = cx.sell_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .sell_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            } else {
                _ = cx.sell(SYMBOL, BASE_QTY).await?;
                _ = cx.buy_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .buy_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            }

            self.ladder = Some(Ladder {
                long_side,
                swing,
                unit_total: BASE_QTY,
                add_count: 0,
                addition_list: [
                    round_to_tick(swing.price_at(ADD1_LEVEL)),
                    round_to_tick(swing.price_at(ADD2_LEVEL)),
                ],
                average_entry: close,
            });
            self.used_origin = Some(swing.origin);

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let max_quantity = result
        .history_position
        .iter()
        .map(|v| v.max_quantity)
        .fold(Decimal::ZERO, Decimal::max);

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  max_drawdown={}  profit/mdd={}  max_ladder_qty={}  long/short={}/{}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        drawdown.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
        max_quantity,
        long_count,
        summary.total_trades - long_count,
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径与 strategy_pullback_trend.rs 对齐：12 个月、4h。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, FibMartingale::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 10. `strategy_pullback_trend`

SMA50/200 trend + RSI pullback entry (**baseline**)。

```rust
use std::collections::VecDeque;
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

struct PullbackStrategy {
    high_buf: VecDeque<Decimal>,
    low_buf: VecDeque<Decimal>,
}

impl PullbackStrategy {
    fn new() -> Self {
        PullbackStrategy {
            high_buf: VecDeque::new(),
            low_buf: VecDeque::new(),
        }
    }
}

#[async_trait(?Send)]
impl Strategy for PullbackStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.high_buf.push_front(cx.high[0]);
        self.low_buf.push_front(cx.low[0]);

        if self.high_buf.len() < 30 {
            return Ok(());
        }

        let sma50 = sma(cx.close, 50);
        let sma200 = sma(cx.close, 200);
        let atr_val = atr(cx.high, cx.low, cx.close, 14);
        let rsi_val = rsi(cx.close, 14);
        let (Some(sma50), Some(sma200), Some(_atr), Some(rsi)) = (sma50, sma200, atr_val, rsi_val)
        else {
            return Ok(());
        };

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        let low: Vec<Decimal> = self.low_buf.iter().copied().collect();
        let high: Vec<Decimal> = self.high_buf.iter().copied().collect();

        // 仅在趋势方向交易
        if sma50 > sma200 {
            let near_sma50 = cx.low[0] <= sma50 * dec!(1.005) && cx.low[0] >= sma50 * dec!(0.99);
            let recent_low = low.iter().take(5).copied().fold(Decimal::MAX, Decimal::min);

            if near_sma50 && cx.close[0] > cx.open[0] && rsi > dec!(40) && rsi < dec!(65) {
                let sl = round_to_tick(recent_low.min(cx.low[0]));
                let tp = round_to_tick(cx.close[0] + (cx.close[0] - sl) * dec!(2));
                cx.cancel_all_order("BTCUSDT").await?;
                _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
                return Ok(());
            }
        }

        if sma50 < sma200 {
            let near_sma50 = cx.high[0] >= sma50 * dec!(0.99) && cx.high[0] <= sma50 * dec!(1.005);
            let recent_high = high
                .iter()
                .take(5)
                .copied()
                .fold(Decimal::MIN, Decimal::max);

            if near_sma50 && cx.close[0] < cx.open[0] && rsi > dec!(35) && rsi < dec!(60) {
                let sl = round_to_tick(recent_high.max(cx.high[0]));
                let tp = round_to_tick(cx.close[0] - (sl - cx.close[0]) * dec!(2));
                cx.cancel_all_order("BTCUSDT").await?;
                _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
            }
        }

        Ok(())
    }
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, PullbackStrategy::new(), Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 11. `strategy_vegas_martingale`

Vegas tunnel + RSI/MACD with martingale adding。

```rust
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

struct VegasMartingale {
    level: usize,
    ema12: EMACache,
    ema144: EMACache,
    ema169: EMACache,
}

impl VegasMartingale {
    fn new() -> Self {
        VegasMartingale {
            level: 0,
            ema12: EMACache::new(12),
            ema144: EMACache::new(144),
            ema169: EMACache::new(169),
        }
    }

    fn qty(&self) -> &str {
        match self.level {
            0 => "0.01",
            1 => "0.02",
            2 => "0.04",
            _ => "0.08",
        }
    }
}

#[async_trait(?Send)]
impl Strategy for VegasMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        if cx.close.len() < 200 {
            return Ok(());
        }

        let Some(ema12) = self.ema12.update(cx.close) else {
            return Ok(());
        };
        let Some(ema144) = self.ema144.update(cx.close) else {
            return Ok(());
        };
        let Some(ema169) = self.ema169.update(cx.close) else {
            return Ok(());
        };

        let rsi = rsi(cx.close, 14);
        let atr = atr(cx.high, cx.low, cx.close, 14);
        let (macd_line, _, histogram) = macd(cx.close, 12, 26, 9);

        let (Some(rsi), Some(atr), Some(_macd), Some(h)) = (rsi, atr, macd_line, histogram) else {
            return Ok(());
        };

        if cx.get_position("BTCUSDT").await?.is_some() {
            return Ok(());
        }

        cx.cancel_all_order("BTCUSDT").await?;

        let price = cx.close[0];

        let band_width = (ema144 - ema169).abs();
        let is_consolidating = band_width < atr * dec!(0.8);

        let trend_up = ema12 > ema144 && ema12 > ema169 && price > ema144 && price > ema169;
        let trend_down = ema12 < ema144 && ema12 < ema169 && price < ema144 && price < ema169;

        if is_consolidating {
            return Ok(());
        }

        if trend_up && h > dec!(0) && rsi > dec!(35) && rsi < dec!(55) {
            let recent_low = cx
                .low
                .iter()
                .take(5)
                .copied()
                .fold(Decimal::MAX, Decimal::min);
            let sl = round_to_tick(recent_low - atr * dec!(0.3));
            let tp = round_to_tick(price + (price - sl) * dec!(2));
            _ = cx.buy_tp_sl("BTCUSDT", tp, sl, self.qty()).await?;
            return Ok(());
        }

        if trend_down && h < dec!(0) && rsi > dec!(45) && rsi < dec!(65) {
            let recent_high = cx
                .high
                .iter()
                .take(5)
                .copied()
                .fold(Decimal::MIN, Decimal::max);
            let sl = round_to_tick(recent_high + atr * dec!(0.3));
            let tp = round_to_tick(price - (sl - price) * dec!(2));
            _ = cx.sell_tp_sl("BTCUSDT", tp, sl, self.qty()).await?;
        }

        Ok(())
    }
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, VegasMartingale::new(), Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 12. `strategy_fib_martingale_050`

Same, entering at the 0.5 retracement。

```rust
//! 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（合约马丁格尔）——**4h 级别 · 0.5 回撤入场**
//!
//! 与 ib_martingale.rs（4h / 0.382）规则完全相同，只把周期换成 1h。
//!
//! 规则完全按设计说明实现，参数不做扫描：
//!
//! 1. **拐点流动性 / 双顶双底**：用 3-3 分形找已确认的拐点。要求最近两个拐点高点
//!    （或低点）价格相近（≤ 1 ATR），构成双顶/双底——这些位置堆积流动性。
//! 2. **斐波那契坐标**：`0 = 顶`，`1 = 底`（做多时反过来 `0 = 底`，`1 = 顶`）。
//!    - 入场：价格回撤到 `0.382`
//!    - 止损基准：`0`（前高/前低），但按解套理论不用它硬止损
//!    - 止盈：`1.382`
//! 3. **真空区/缺口**：要求这段摆动里至少有一根实体 ≥ 1 ATR 的急促 K 线。
//!    这种价位没有像样的止损盘，反向砸起来也快，所以延续概率更高。
//! 4. **解套理论 + 马丁格尔**：入场后价格不但没回来，反而突破前高（做空方向即 `0` 位），
//!    按 `前高 + 0.1R` 加 0.01、`前高 + 0.2R` 再加 0.02，最多加两次（总 0.04）。
//!    加仓把均价拉到前高附近，等价格回抽前高时**保本/解套离场**；
//!    第三次加仓之后若继续逆行，用 `前高 + 1 ATR` 的硬止损兜底。
//!
//! 注意：马丁格尔让单笔最大仓位到 0.04（基础单的 4 倍），
//! 所以绝对利润不能和前面几个固定 0.01 手位的策略直接横向比较。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;

/// 基础单与两次加仓的仓位
const BASE_QTY: Decimal = dec!(0.01);
const ADD1_QTY: Decimal = dec!(0.01);
const ADD2_QTY: Decimal = dec!(0.02);

/// 分形拐点的左右宽度
const PIVOT_SPAN: usize = 3;
/// 双顶/双底的价格相近度（ATR 倍数）
const DOUBLE_TOL_ATR: Decimal = dec!(1.0);
/// 摆动最小幅度（ATR 倍数），太小的摆动没有交易价值
const MIN_RANGE_ATR: Decimal = dec!(2.0);
/// 真空区判定：摆动内最大实体至少这么多 ATR
const VACUUM_BODY_ATR: Decimal = dec!(1.0);
/// 回撤/止盈/加仓的斐波那契位
const ENTRY_LEVEL: Decimal = dec!(0.5);
const TAKE_LEVEL: Decimal = dec!(1.382);
const ADD1_LEVEL: Decimal = dec!(-0.1);
const ADD2_LEVEL: Decimal = dec!(-0.2);
/// 三次加仓后的硬止损（ATR 倍数，位于 0 位之外）
const DISASTER_STOP_ATR: Decimal = dec!(1.0);

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// 一根 K 线的快照
#[derive(Clone)]
struct Candle {
    high: Decimal,
    low: Decimal,
    open: Decimal,
    close: Decimal,
}

impl Candle {
    fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }
}

/// 一个正在跟踪的斐波那契摆动
#[derive(Clone, Copy)]
struct Swing {
    /// 0 位价格（双顶的最高点 / 双底的最低点）
    origin: Decimal,
    /// 1 位价格（对面摆动极值）
    opposite: Decimal,
}

impl Swing {
    fn range(&self) -> Decimal {
        (self.origin - self.opposite).abs()
    }

    /// 按 0=顶、1=底 的坐标取价位；做多时 origin 是底，方向自动反过来。
    fn price_at(&self, level: Decimal) -> Decimal {
        self.origin + (self.opposite - self.origin) * level
    }
}

/// 一次持仓的马丁格尔阶梯
struct Ladder {
    long_side: bool,
    swing: Swing,
    unit_total: Decimal,
    add_count: usize,
    addition_list: [Decimal; 2],
    average_entry: Decimal,
}

struct FibMartingale {
    candle_list: VecDeque<Candle>,
    close_cache: VecDeque<Decimal>,
    bar_count: usize,
    ladder: Option<Ladder>,
    used_origin: Option<Decimal>,
}

impl FibMartingale {
    fn new() -> Self {
        FibMartingale {
            candle_list: VecDeque::new(),
            close_cache: VecDeque::new(),
            bar_count: 0,
            ladder: None,
            used_origin: None,
        }
    }

    /// 在 newest-first 的缓冲里找第 `skip` 个已确认拐点高点（返回索引）。
    fn find_pivot_high(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].high;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].high < value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].high < value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    fn find_pivot_low(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].low;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].low > value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].low > value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    /// 摆动区间内是否出现过急促实体（真空区）。
    fn has_vacuum(&self, from: usize, to: usize, atr: Decimal) -> bool {
        if from <= to {
            return false;
        }

        let threshold = VACUUM_BODY_ATR * atr;

        (to..from).any(|i| self.candle_list[i].body() >= threshold)
    }

    /// 找最近的“双顶”，返回摆动（0 = 顶，1 = 底）。
    fn find_double_top(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_high(0)?;
        let previous = self.find_pivot_high(1)?;
        let top = self.candle_list[last].high;
        let top_previous = self.candle_list[previous].high;

        if (top - top_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let bottom = (0..last)
            .map(|i| self.candle_list[i].low)
            .fold(Decimal::MAX, Decimal::min);
        let swing = Swing {
            origin: top,
            opposite: bottom,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }

    /// 找最近的“双底”，返回摆动（0 = 底，1 = 顶）。
    fn find_double_bottom(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_low(0)?;
        let previous = self.find_pivot_low(1)?;
        let bottom = self.candle_list[last].low;
        let bottom_previous = self.candle_list[previous].low;

        if (bottom - bottom_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let top = (0..last)
            .map(|i| self.candle_list[i].high)
            .fold(Decimal::MIN, Decimal::max);
        let swing = Swing {
            origin: bottom,
            opposite: top,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }
}

#[async_trait(?Send)]
impl Strategy for FibMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        self.candle_list.push_front(Candle {
            high,
            low,
            open,
            close,
        });

        if self.candle_list.len() > 80 {
            self.candle_list.truncate(80);
        }

        self.close_cache.push_front(close);

        if self.close_cache.len() > 200 {
            self.close_cache.truncate(200);
        }

        self.bar_count += 1;

        if self.bar_count < 60 {
            return Ok(());
        }

        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        let position = cx.get_position(SYMBOL).await?;

        // ---------------- 持仓管理：加仓 / 解套 / 硬止损 ----------------
        if position.is_some() {
            let Some(ladder) = self.ladder.as_mut() else {
                return Ok(());
            };

            let long_side = ladder.long_side;
            let origin = ladder.swing.origin;

            // 硬止损：0 位之外 1 ATR（由交易所的止损单兜底，这里只做状态清理）
            let disaster = if long_side {
                round_to_tick(origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(origin + DISASTER_STOP_ATR * atr)
            };
            let hit_disaster = if long_side {
                low <= disaster
            } else {
                high >= disaster
            };

            // 解套：已经加过仓，价格回抽 0 位（前高/前低）→ 保本离场
            let escape = ladder.add_count > 0
                && if long_side {
                    high >= origin
                } else {
                    low <= origin
                };

            // 加仓：价格突破 0 位一段距离
            let next_add = if ladder.add_count < ladder.addition_list.len() {
                Some(ladder.addition_list[ladder.add_count])
            } else {
                None
            };
            let need_add = next_add
                .map(|price| {
                    if long_side {
                        low <= price
                    } else {
                        high >= price
                    }
                })
                .unwrap_or(false);

            if need_add {
                let add_quantity = if ladder.add_count == 0 {
                    ADD1_QTY
                } else {
                    ADD2_QTY
                };
                let price = next_add.unwrap();

                // 用挂单价估算均价；实际成交价由撮合决定，方向一致即可
                let previous_notional = ladder.average_entry * ladder.unit_total;
                let new_total = ladder.unit_total + add_quantity;

                ladder.average_entry = (previous_notional + price * add_quantity) / new_total;
                ladder.unit_total = new_total;
                ladder.add_count += 1;

                let total = ladder.unit_total;
                let take = round_to_tick(ladder.swing.price_at(TAKE_LEVEL));

                // 必须先撤旧单，再下加仓单：cancel_all_order 会把刚挂出的市价单一起撤掉
                cx.cancel_all_order(SYMBOL).await?;

                if long_side {
                    _ = cx.buy(SYMBOL, add_quantity).await?;
                } else {
                    _ = cx.sell(SYMBOL, add_quantity).await?;
                }

                // 仓位变了，止盈止损要按新的总仓位重挂
                if long_side {
                    _ = cx.sell_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .sell_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                } else {
                    _ = cx.buy_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .buy_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                }

                return Ok(());
            }

            if hit_disaster || escape {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.ladder = None;
            }

            return Ok(());
        }

        self.ladder = None;

        // ---------------- 入场 ----------------
        let long_setup = self.find_double_bottom(atr);
        let short_setup = self.find_double_top(atr);

        // 先看空（双顶）再看多（双底）；同一根只做一笔
        for (long_side, swing) in [(false, short_setup), (true, long_setup)] {
            let Some(swing) = swing else {
                continue;
            };

            if self.used_origin == Some(swing.origin) {
                continue;
            }

            let entry_level = swing.price_at(ENTRY_LEVEL);
            let reached = if long_side {
                low <= entry_level
            } else {
                high >= entry_level
            };

            // 必须仍然在 0 位之内（还没被突破），否则这套结构已经失效
            let inside = if long_side {
                close > swing.origin
            } else {
                close < swing.origin
            };

            if !reached || !inside {
                continue;
            }

            let take = round_to_tick(swing.price_at(TAKE_LEVEL));
            let disaster = if long_side {
                round_to_tick(swing.origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(swing.origin + DISASTER_STOP_ATR * atr)
            };

            if (long_side && take <= close) || (!long_side && take >= close) {
                continue;
            }

            cx.cancel_all_order(SYMBOL).await?;

            if long_side {
                _ = cx.buy(SYMBOL, BASE_QTY).await?;
                _ = cx.sell_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .sell_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            } else {
                _ = cx.sell(SYMBOL, BASE_QTY).await?;
                _ = cx.buy_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .buy_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            }

            self.ladder = Some(Ladder {
                long_side,
                swing,
                unit_total: BASE_QTY,
                add_count: 0,
                addition_list: [
                    round_to_tick(swing.price_at(ADD1_LEVEL)),
                    round_to_tick(swing.price_at(ADD2_LEVEL)),
                ],
                average_entry: close,
            });
            self.used_origin = Some(swing.origin);

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let max_quantity = result
        .history_position
        .iter()
        .map(|v| v.max_quantity)
        .fold(Decimal::ZERO, Decimal::max);

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  max_drawdown={}  profit/mdd={}  max_ladder_qty={}  long/short={}/{}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        drawdown.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
        max_quantity,
        long_count,
        summary.total_trades - long_count,
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径：12 个月；24/36/48 个月做稳健性检查（参数未按窗口调整）。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, FibMartingale::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 13. `strategy_shadow_reversal`

Long-wick reversal with ATR-based take-profit/stop。

```rust
use trading_maid::prelude::*;

fn round_to_tick(price: Decimal) -> Decimal {
    let tick = dec!(0.1);
    let rounded = (price / tick).round_dp(0) * tick;
    if rounded <= Decimal::ZERO {
        tick
    } else {
        rounded
    }
}

async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if cx.close.len() < 50 {
        return Ok(());
    }

    let atr_val = atr(cx.high, cx.low, cx.close, 14);
    let Some(atr) = atr_val else { return Ok(()) };

    if cx.get_position("BTCUSDT").await?.is_some() {
        return Ok(());
    }

    let body = (cx.open[0] - cx.close[0]).abs();

    // 长上影线做空：上影线 >= 实体 2 倍
    let upper_shadow = cx.high[0] - cx.open[0].max(cx.close[0]);
    if cx.close[0] < cx.open[0] && upper_shadow >= body * dec!(2) && body >= dec!(200) {
        let sl = round_to_tick(cx.high[0] + atr * dec!(0.5));
        let tp = round_to_tick(cx.low[0] - atr * dec!(3));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.sell_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
        return Ok(());
    }

    // 长下影线做多：下影线 >= 实体 2 倍
    let lower_shadow = cx.open[0].min(cx.close[0]) - cx.low[0];
    if cx.close[0] > cx.open[0] && lower_shadow >= body * dec!(2) && body >= dec!(200) {
        let sl = round_to_tick(cx.low[0] - atr * dec!(0.5));
        let tp = round_to_tick(cx.high[0] + atr * dec!(3));
        cx.cancel_all_order("BTCUSDT").await?;
        _ = cx.buy_tp_sl("BTCUSDT", tp, sl, "0.01").await?;
    }

    Ok(())
}


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, my_strategy, Level::Hour4)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour4, &result);
    }
}
```

### 14. `strategy_fib_martingale_1h_050`

Same, 1h + 0.5 retracement。

```rust
//! 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（合约马丁格尔）——**1 小时级别 · 0.5 回撤入场**
//!
//! 与 ib_martingale.rs（4h / 0.382）规则完全相同，只把周期换成 1h。
//!
//! 规则完全按设计说明实现，参数不做扫描：
//!
//! 1. **拐点流动性 / 双顶双底**：用 3-3 分形找已确认的拐点。要求最近两个拐点高点
//!    （或低点）价格相近（≤ 1 ATR），构成双顶/双底——这些位置堆积流动性。
//! 2. **斐波那契坐标**：`0 = 顶`，`1 = 底`（做多时反过来 `0 = 底`，`1 = 顶`）。
//!    - 入场：价格回撤到 `0.382`
//!    - 止损基准：`0`（前高/前低），但按解套理论不用它硬止损
//!    - 止盈：`1.382`
//! 3. **真空区/缺口**：要求这段摆动里至少有一根实体 ≥ 1 ATR 的急促 K 线。
//!    这种价位没有像样的止损盘，反向砸起来也快，所以延续概率更高。
//! 4. **解套理论 + 马丁格尔**：入场后价格不但没回来，反而突破前高（做空方向即 `0` 位），
//!    按 `前高 + 0.1R` 加 0.01、`前高 + 0.2R` 再加 0.02，最多加两次（总 0.04）。
//!    加仓把均价拉到前高附近，等价格回抽前高时**保本/解套离场**；
//!    第三次加仓之后若继续逆行，用 `前高 + 1 ATR` 的硬止损兜底。
//!
//! 注意：马丁格尔让单笔最大仓位到 0.04（基础单的 4 倍），
//! 所以绝对利润不能和前面几个固定 0.01 手位的策略直接横向比较。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour1;
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;

/// 基础单与两次加仓的仓位
const BASE_QTY: Decimal = dec!(0.01);
const ADD1_QTY: Decimal = dec!(0.01);
const ADD2_QTY: Decimal = dec!(0.02);

/// 分形拐点的左右宽度
const PIVOT_SPAN: usize = 3;
/// 双顶/双底的价格相近度（ATR 倍数）
const DOUBLE_TOL_ATR: Decimal = dec!(1.0);
/// 摆动最小幅度（ATR 倍数），太小的摆动没有交易价值
const MIN_RANGE_ATR: Decimal = dec!(2.0);
/// 真空区判定：摆动内最大实体至少这么多 ATR
const VACUUM_BODY_ATR: Decimal = dec!(1.0);
/// 回撤/止盈/加仓的斐波那契位
const ENTRY_LEVEL: Decimal = dec!(0.5);
const TAKE_LEVEL: Decimal = dec!(1.382);
const ADD1_LEVEL: Decimal = dec!(-0.1);
const ADD2_LEVEL: Decimal = dec!(-0.2);
/// 三次加仓后的硬止损（ATR 倍数，位于 0 位之外）
const DISASTER_STOP_ATR: Decimal = dec!(1.0);

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// 一根 K 线的快照
#[derive(Clone)]
struct Candle {
    high: Decimal,
    low: Decimal,
    open: Decimal,
    close: Decimal,
}

impl Candle {
    fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }
}

/// 一个正在跟踪的斐波那契摆动
#[derive(Clone, Copy)]
struct Swing {
    /// 0 位价格（双顶的最高点 / 双底的最低点）
    origin: Decimal,
    /// 1 位价格（对面摆动极值）
    opposite: Decimal,
}

impl Swing {
    fn range(&self) -> Decimal {
        (self.origin - self.opposite).abs()
    }

    /// 按 0=顶、1=底 的坐标取价位；做多时 origin 是底，方向自动反过来。
    fn price_at(&self, level: Decimal) -> Decimal {
        self.origin + (self.opposite - self.origin) * level
    }
}

/// 一次持仓的马丁格尔阶梯
struct Ladder {
    long_side: bool,
    swing: Swing,
    unit_total: Decimal,
    add_count: usize,
    addition_list: [Decimal; 2],
    average_entry: Decimal,
}

struct FibMartingale {
    candle_list: VecDeque<Candle>,
    close_cache: VecDeque<Decimal>,
    bar_count: usize,
    ladder: Option<Ladder>,
    used_origin: Option<Decimal>,
}

impl FibMartingale {
    fn new() -> Self {
        FibMartingale {
            candle_list: VecDeque::new(),
            close_cache: VecDeque::new(),
            bar_count: 0,
            ladder: None,
            used_origin: None,
        }
    }

    /// 在 newest-first 的缓冲里找第 `skip` 个已确认拐点高点（返回索引）。
    fn find_pivot_high(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].high;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].high < value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].high < value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    fn find_pivot_low(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].low;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].low > value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].low > value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    /// 摆动区间内是否出现过急促实体（真空区）。
    fn has_vacuum(&self, from: usize, to: usize, atr: Decimal) -> bool {
        if from <= to {
            return false;
        }

        let threshold = VACUUM_BODY_ATR * atr;

        (to..from).any(|i| self.candle_list[i].body() >= threshold)
    }

    /// 找最近的“双顶”，返回摆动（0 = 顶，1 = 底）。
    fn find_double_top(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_high(0)?;
        let previous = self.find_pivot_high(1)?;
        let top = self.candle_list[last].high;
        let top_previous = self.candle_list[previous].high;

        if (top - top_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let bottom = (0..last)
            .map(|i| self.candle_list[i].low)
            .fold(Decimal::MAX, Decimal::min);
        let swing = Swing {
            origin: top,
            opposite: bottom,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }

    /// 找最近的“双底”，返回摆动（0 = 底，1 = 顶）。
    fn find_double_bottom(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_low(0)?;
        let previous = self.find_pivot_low(1)?;
        let bottom = self.candle_list[last].low;
        let bottom_previous = self.candle_list[previous].low;

        if (bottom - bottom_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let top = (0..last)
            .map(|i| self.candle_list[i].high)
            .fold(Decimal::MIN, Decimal::max);
        let swing = Swing {
            origin: bottom,
            opposite: top,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }
}

#[async_trait(?Send)]
impl Strategy for FibMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        self.candle_list.push_front(Candle {
            high,
            low,
            open,
            close,
        });

        if self.candle_list.len() > 80 {
            self.candle_list.truncate(80);
        }

        self.close_cache.push_front(close);

        if self.close_cache.len() > 200 {
            self.close_cache.truncate(200);
        }

        self.bar_count += 1;

        if self.bar_count < 60 {
            return Ok(());
        }

        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        let position = cx.get_position(SYMBOL).await?;

        // ---------------- 持仓管理：加仓 / 解套 / 硬止损 ----------------
        if position.is_some() {
            let Some(ladder) = self.ladder.as_mut() else {
                return Ok(());
            };

            let long_side = ladder.long_side;
            let origin = ladder.swing.origin;

            // 硬止损：0 位之外 1 ATR（由交易所的止损单兜底，这里只做状态清理）
            let disaster = if long_side {
                round_to_tick(origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(origin + DISASTER_STOP_ATR * atr)
            };
            let hit_disaster = if long_side {
                low <= disaster
            } else {
                high >= disaster
            };

            // 解套：已经加过仓，价格回抽 0 位（前高/前低）→ 保本离场
            let escape = ladder.add_count > 0
                && if long_side {
                    high >= origin
                } else {
                    low <= origin
                };

            // 加仓：价格突破 0 位一段距离
            let next_add = if ladder.add_count < ladder.addition_list.len() {
                Some(ladder.addition_list[ladder.add_count])
            } else {
                None
            };
            let need_add = next_add
                .map(|price| {
                    if long_side {
                        low <= price
                    } else {
                        high >= price
                    }
                })
                .unwrap_or(false);

            if need_add {
                let add_quantity = if ladder.add_count == 0 {
                    ADD1_QTY
                } else {
                    ADD2_QTY
                };
                let price = next_add.unwrap();

                // 用挂单价估算均价；实际成交价由撮合决定，方向一致即可
                let previous_notional = ladder.average_entry * ladder.unit_total;
                let new_total = ladder.unit_total + add_quantity;

                ladder.average_entry = (previous_notional + price * add_quantity) / new_total;
                ladder.unit_total = new_total;
                ladder.add_count += 1;

                let total = ladder.unit_total;
                let take = round_to_tick(ladder.swing.price_at(TAKE_LEVEL));

                // 必须先撤旧单，再下加仓单：cancel_all_order 会把刚挂出的市价单一起撤掉
                cx.cancel_all_order(SYMBOL).await?;

                if long_side {
                    _ = cx.buy(SYMBOL, add_quantity).await?;
                } else {
                    _ = cx.sell(SYMBOL, add_quantity).await?;
                }

                // 仓位变了，止盈止损要按新的总仓位重挂
                if long_side {
                    _ = cx.sell_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .sell_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                } else {
                    _ = cx.buy_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .buy_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                }

                return Ok(());
            }

            if hit_disaster || escape {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.ladder = None;
            }

            return Ok(());
        }

        self.ladder = None;

        // ---------------- 入场 ----------------
        let long_setup = self.find_double_bottom(atr);
        let short_setup = self.find_double_top(atr);

        // 先看空（双顶）再看多（双底）；同一根只做一笔
        for (long_side, swing) in [(false, short_setup), (true, long_setup)] {
            let Some(swing) = swing else {
                continue;
            };

            if self.used_origin == Some(swing.origin) {
                continue;
            }

            let entry_level = swing.price_at(ENTRY_LEVEL);
            let reached = if long_side {
                low <= entry_level
            } else {
                high >= entry_level
            };

            // 必须仍然在 0 位之内（还没被突破），否则这套结构已经失效
            let inside = if long_side {
                close > swing.origin
            } else {
                close < swing.origin
            };

            if !reached || !inside {
                continue;
            }

            let take = round_to_tick(swing.price_at(TAKE_LEVEL));
            let disaster = if long_side {
                round_to_tick(swing.origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(swing.origin + DISASTER_STOP_ATR * atr)
            };

            if (long_side && take <= close) || (!long_side && take >= close) {
                continue;
            }

            cx.cancel_all_order(SYMBOL).await?;

            if long_side {
                _ = cx.buy(SYMBOL, BASE_QTY).await?;
                _ = cx.sell_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .sell_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            } else {
                _ = cx.sell(SYMBOL, BASE_QTY).await?;
                _ = cx.buy_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .buy_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            }

            self.ladder = Some(Ladder {
                long_side,
                swing,
                unit_total: BASE_QTY,
                add_count: 0,
                addition_list: [
                    round_to_tick(swing.price_at(ADD1_LEVEL)),
                    round_to_tick(swing.price_at(ADD2_LEVEL)),
                ],
                average_entry: close,
            });
            self.used_origin = Some(swing.origin);

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let max_quantity = result
        .history_position
        .iter()
        .map(|v| v.max_quantity)
        .fold(Decimal::ZERO, Decimal::max);

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  max_drawdown={}  profit/mdd={}  max_ladder_qty={}  long/short={}/{}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        drawdown.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
        max_quantity,
        long_count,
        summary.total_trades - long_count,
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径：12 个月；24/36/48 个月做稳健性检查（参数未按窗口调整）。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, FibMartingale::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 15. `strategy_fib_martingale_1h`

Same, moved to 1h。

```rust
//! 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（合约马丁格尔）——**1 小时级别**
//!
//! 与 ib_martingale.rs（4h / 0.382）规则完全相同，只把周期换成 1h。
//!
//! 规则完全按设计说明实现，参数不做扫描：
//!
//! 1. **拐点流动性 / 双顶双底**：用 3-3 分形找已确认的拐点。要求最近两个拐点高点
//!    （或低点）价格相近（≤ 1 ATR），构成双顶/双底——这些位置堆积流动性。
//! 2. **斐波那契坐标**：`0 = 顶`，`1 = 底`（做多时反过来 `0 = 底`，`1 = 顶`）。
//!    - 入场：价格回撤到 `0.382`
//!    - 止损基准：`0`（前高/前低），但按解套理论不用它硬止损
//!    - 止盈：`1.382`
//! 3. **真空区/缺口**：要求这段摆动里至少有一根实体 ≥ 1 ATR 的急促 K 线。
//!    这种价位没有像样的止损盘，反向砸起来也快，所以延续概率更高。
//! 4. **解套理论 + 马丁格尔**：入场后价格不但没回来，反而突破前高（做空方向即 `0` 位），
//!    按 `前高 + 0.1R` 加 0.01、`前高 + 0.2R` 再加 0.02，最多加两次（总 0.04）。
//!    加仓把均价拉到前高附近，等价格回抽前高时**保本/解套离场**；
//!    第三次加仓之后若继续逆行，用 `前高 + 1 ATR` 的硬止损兜底。
//!
//! 注意：马丁格尔让单笔最大仓位到 0.04（基础单的 4 倍），
//! 所以绝对利润不能和前面几个固定 0.01 手位的策略直接横向比较。

use std::collections::VecDeque;
use trading_maid::prelude::*;

const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour1;
const TICK: Decimal = dec!(0.1);
const ATR_N: usize = 14;

/// 基础单与两次加仓的仓位
const BASE_QTY: Decimal = dec!(0.01);
const ADD1_QTY: Decimal = dec!(0.01);
const ADD2_QTY: Decimal = dec!(0.02);

/// 分形拐点的左右宽度
const PIVOT_SPAN: usize = 3;
/// 双顶/双底的价格相近度（ATR 倍数）
const DOUBLE_TOL_ATR: Decimal = dec!(1.0);
/// 摆动最小幅度（ATR 倍数），太小的摆动没有交易价值
const MIN_RANGE_ATR: Decimal = dec!(2.0);
/// 真空区判定：摆动内最大实体至少这么多 ATR
const VACUUM_BODY_ATR: Decimal = dec!(1.0);
/// 回撤/止盈/加仓的斐波那契位
const ENTRY_LEVEL: Decimal = dec!(0.382);
const TAKE_LEVEL: Decimal = dec!(1.382);
const ADD1_LEVEL: Decimal = dec!(-0.1);
const ADD2_LEVEL: Decimal = dec!(-0.2);
/// 三次加仓后的硬止损（ATR 倍数，位于 0 位之外）
const DISASTER_STOP_ATR: Decimal = dec!(1.0);

fn round_to_tick(price: Decimal) -> Decimal {
    let rounded = (price / TICK).round_dp(0) * TICK;

    if rounded <= Decimal::ZERO {
        TICK
    } else {
        rounded
    }
}

/// 一根 K 线的快照
#[derive(Clone)]
struct Candle {
    high: Decimal,
    low: Decimal,
    open: Decimal,
    close: Decimal,
}

impl Candle {
    fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }
}

/// 一个正在跟踪的斐波那契摆动
#[derive(Clone, Copy)]
struct Swing {
    /// 0 位价格（双顶的最高点 / 双底的最低点）
    origin: Decimal,
    /// 1 位价格（对面摆动极值）
    opposite: Decimal,
}

impl Swing {
    fn range(&self) -> Decimal {
        (self.origin - self.opposite).abs()
    }

    /// 按 0=顶、1=底 的坐标取价位；做多时 origin 是底，方向自动反过来。
    fn price_at(&self, level: Decimal) -> Decimal {
        self.origin + (self.opposite - self.origin) * level
    }
}

/// 一次持仓的马丁格尔阶梯
struct Ladder {
    long_side: bool,
    swing: Swing,
    unit_total: Decimal,
    add_count: usize,
    addition_list: [Decimal; 2],
    average_entry: Decimal,
}

struct FibMartingale {
    candle_list: VecDeque<Candle>,
    close_cache: VecDeque<Decimal>,
    bar_count: usize,
    ladder: Option<Ladder>,
    used_origin: Option<Decimal>,
}

impl FibMartingale {
    fn new() -> Self {
        FibMartingale {
            candle_list: VecDeque::new(),
            close_cache: VecDeque::new(),
            bar_count: 0,
            ladder: None,
            used_origin: None,
        }
    }

    /// 在 newest-first 的缓冲里找第 `skip` 个已确认拐点高点（返回索引）。
    fn find_pivot_high(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].high;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].high < value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].high < value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    fn find_pivot_low(&self, skip: usize) -> Option<usize> {
        let candle_list = &self.candle_list;
        let len = candle_list.len();

        if len < PIVOT_SPAN * 2 + 1 {
            return None;
        }

        let mut found = 0;

        for i in (PIVOT_SPAN + 1)..(len - PIVOT_SPAN) {
            let value = candle_list[i].low;
            let left_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i + k].low > value);
            let right_ok = (1..=PIVOT_SPAN).all(|k| candle_list[i - k].low > value);

            if left_ok && right_ok {
                found += 1;

                if found > skip {
                    return Some(i);
                }
            }
        }

        None
    }

    /// 摆动区间内是否出现过急促实体（真空区）。
    fn has_vacuum(&self, from: usize, to: usize, atr: Decimal) -> bool {
        if from <= to {
            return false;
        }

        let threshold = VACUUM_BODY_ATR * atr;

        (to..from).any(|i| self.candle_list[i].body() >= threshold)
    }

    /// 找最近的“双顶”，返回摆动（0 = 顶，1 = 底）。
    fn find_double_top(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_high(0)?;
        let previous = self.find_pivot_high(1)?;
        let top = self.candle_list[last].high;
        let top_previous = self.candle_list[previous].high;

        if (top - top_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let bottom = (0..last)
            .map(|i| self.candle_list[i].low)
            .fold(Decimal::MAX, Decimal::min);
        let swing = Swing {
            origin: top,
            opposite: bottom,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }

    /// 找最近的“双底”，返回摆动（0 = 底，1 = 顶）。
    fn find_double_bottom(&self, atr: Decimal) -> Option<Swing> {
        let last = self.find_pivot_low(0)?;
        let previous = self.find_pivot_low(1)?;
        let bottom = self.candle_list[last].low;
        let bottom_previous = self.candle_list[previous].low;

        if (bottom - bottom_previous).abs() > DOUBLE_TOL_ATR * atr {
            return None;
        }

        let top = (0..last)
            .map(|i| self.candle_list[i].high)
            .fold(Decimal::MIN, Decimal::max);
        let swing = Swing {
            origin: bottom,
            opposite: top,
        };

        if swing.range() < MIN_RANGE_ATR * atr {
            return None;
        }

        if !self.has_vacuum(last, 0, atr) {
            return None;
        }

        Some(swing)
    }
}

#[async_trait(?Send)]
impl Strategy for FibMartingale {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let open = cx.open[0];
        let high = cx.high[0];
        let low = cx.low[0];
        let close = cx.close[0];

        self.candle_list.push_front(Candle {
            high,
            low,
            open,
            close,
        });

        if self.candle_list.len() > 80 {
            self.candle_list.truncate(80);
        }

        self.close_cache.push_front(close);

        if self.close_cache.len() > 200 {
            self.close_cache.truncate(200);
        }

        self.bar_count += 1;

        if self.bar_count < 60 {
            return Ok(());
        }

        let Some(atr) = atr(cx.high, cx.low, cx.close, ATR_N) else {
            return Ok(());
        };

        if atr.is_zero() {
            return Ok(());
        }

        let position = cx.get_position(SYMBOL).await?;

        // ---------------- 持仓管理：加仓 / 解套 / 硬止损 ----------------
        if position.is_some() {
            let Some(ladder) = self.ladder.as_mut() else {
                return Ok(());
            };

            let long_side = ladder.long_side;
            let origin = ladder.swing.origin;

            // 硬止损：0 位之外 1 ATR（由交易所的止损单兜底，这里只做状态清理）
            let disaster = if long_side {
                round_to_tick(origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(origin + DISASTER_STOP_ATR * atr)
            };
            let hit_disaster = if long_side {
                low <= disaster
            } else {
                high >= disaster
            };

            // 解套：已经加过仓，价格回抽 0 位（前高/前低）→ 保本离场
            let escape = ladder.add_count > 0
                && if long_side {
                    high >= origin
                } else {
                    low <= origin
                };

            // 加仓：价格突破 0 位一段距离
            let next_add = if ladder.add_count < ladder.addition_list.len() {
                Some(ladder.addition_list[ladder.add_count])
            } else {
                None
            };
            let need_add = next_add
                .map(|price| {
                    if long_side {
                        low <= price
                    } else {
                        high >= price
                    }
                })
                .unwrap_or(false);

            if need_add {
                let add_quantity = if ladder.add_count == 0 {
                    ADD1_QTY
                } else {
                    ADD2_QTY
                };
                let price = next_add.unwrap();

                // 用挂单价估算均价；实际成交价由撮合决定，方向一致即可
                let previous_notional = ladder.average_entry * ladder.unit_total;
                let new_total = ladder.unit_total + add_quantity;

                ladder.average_entry = (previous_notional + price * add_quantity) / new_total;
                ladder.unit_total = new_total;
                ladder.add_count += 1;

                let total = ladder.unit_total;
                let take = round_to_tick(ladder.swing.price_at(TAKE_LEVEL));

                // 必须先撤旧单，再下加仓单：cancel_all_order 会把刚挂出的市价单一起撤掉
                cx.cancel_all_order(SYMBOL).await?;

                if long_side {
                    _ = cx.buy(SYMBOL, add_quantity).await?;
                } else {
                    _ = cx.sell(SYMBOL, add_quantity).await?;
                }

                // 仓位变了，止盈止损要按新的总仓位重挂
                if long_side {
                    _ = cx.sell_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .sell_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                } else {
                    _ = cx.buy_limit_reduce_only(SYMBOL, take, total).await?;
                    _ = cx
                        .buy_trigger_market_reduce_only(SYMBOL, disaster, total)
                        .await?;
                }

                return Ok(());
            }

            if hit_disaster || escape {
                cx.cancel_all_order(SYMBOL).await?;
                cx.close_all_position(SYMBOL).await?;
                self.ladder = None;
            }

            return Ok(());
        }

        self.ladder = None;

        // ---------------- 入场 ----------------
        let long_setup = self.find_double_bottom(atr);
        let short_setup = self.find_double_top(atr);

        // 先看空（双顶）再看多（双底）；同一根只做一笔
        for (long_side, swing) in [(false, short_setup), (true, long_setup)] {
            let Some(swing) = swing else {
                continue;
            };

            if self.used_origin == Some(swing.origin) {
                continue;
            }

            let entry_level = swing.price_at(ENTRY_LEVEL);
            let reached = if long_side {
                low <= entry_level
            } else {
                high >= entry_level
            };

            // 必须仍然在 0 位之内（还没被突破），否则这套结构已经失效
            let inside = if long_side {
                close > swing.origin
            } else {
                close < swing.origin
            };

            if !reached || !inside {
                continue;
            }

            let take = round_to_tick(swing.price_at(TAKE_LEVEL));
            let disaster = if long_side {
                round_to_tick(swing.origin - DISASTER_STOP_ATR * atr)
            } else {
                round_to_tick(swing.origin + DISASTER_STOP_ATR * atr)
            };

            if (long_side && take <= close) || (!long_side && take >= close) {
                continue;
            }

            cx.cancel_all_order(SYMBOL).await?;

            if long_side {
                _ = cx.buy(SYMBOL, BASE_QTY).await?;
                _ = cx.sell_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .sell_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            } else {
                _ = cx.sell(SYMBOL, BASE_QTY).await?;
                _ = cx.buy_limit_reduce_only(SYMBOL, take, BASE_QTY).await?;
                _ = cx
                    .buy_trigger_market_reduce_only(SYMBOL, disaster, BASE_QTY)
                    .await?;
            }

            self.ladder = Some(Ladder {
                long_side,
                swing,
                unit_total: BASE_QTY,
                add_count: 0,
                addition_list: [
                    round_to_tick(swing.price_at(ADD1_LEVEL)),
                    round_to_tick(swing.price_at(ADD2_LEVEL)),
                ],
                average_entry: close,
            });
            self.used_origin = Some(swing.origin);

            return Ok(());
        }

        Ok(())
    }
}

/// 逐笔累计的权益回撤（按平仓时间排序）。
fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let max_quantity = result
        .history_position
        .iter()
        .map(|v| v.max_quantity)
        .fold(Decimal::ZERO, Decimal::max);

    println!("== {} 个月 · {} · {} ==", month_count, SYMBOL, LEVEL);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  max_drawdown={}  profit/mdd={}  max_ladder_qty={}  long/short={}/{}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        drawdown.round_dp(2),
        if drawdown.is_zero() {
            Decimal::ZERO
        } else {
            (summary.total_profit / drawdown).round_dp(2)
        },
        max_quantity,
        long_count,
        summary.total_trades - long_count,
    );
    println!();
}

#[tokio::main]
async fn main() {
    // 主口径：12 个月；24/36/48 个月做稳健性检查（参数未按窗口调整）。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest(SYMBOL, month_count, FibMartingale::new(), LEVEL)
            .await
            .unwrap();

        print_report(month_count, &result);
    }
}
```

### 16. `strategy_fast`

Short on a long upper wick — the simplest rule set here。

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


fn max_drawdown(history: &[HistoryPosition]) -> Decimal {
    let mut position_list = history.iter().collect::<Vec<_>>();

    position_list.sort_by_key(|v| v.close_time);

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut drawdown = Decimal::ZERO;

    for v in position_list {
        equity += v.total_profit;

        if equity > peak {
            peak = equity;
        }

        if peak - equity > drawdown {
            drawdown = peak - equity;
        }
    }

    drawdown
}

fn print_report(month_count: u32, symbol: &str, level: Level, result: &BacktestResult) {
    let summary = result.summarize();
    let drawdown = max_drawdown(&result.history_position);
    let profit_factor = if summary.gross_loss_abs.is_zero() {
        Decimal::ZERO
    } else {
        summary.gross_profit / summary.gross_loss_abs
    };
    let fee_ratio = if summary.total_profit.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fee / summary.total_profit * dec!(100)
    };
    let long_count = result
        .history_position
        .iter()
        .filter(|v| v.side == Side::Buy)
        .count();
    let short_count = summary.total_trades - long_count;

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "trades={}  win_rate={}%  profit_loss_ratio={}  profit_factor_gross={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        profit_factor.round_dp(2),
    );
    println!(
        "total_profit={}  total_fee={}  fee/profit={}%  max_drawdown={}  avg_profit/trade={}",
        summary.total_profit.round_dp(2),
        summary.total_fee.round_dp(2),
        fee_ratio.round_dp(2),
        drawdown.round_dp(2),
        summary.avg_profit.round_dp(2),
    );
    println!("long/short = {}/{}", long_count, short_count);
    println!();
}
#[tokio::main]
async fn main() {
    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let result = backtest("BTCUSDT", month_count, my_strategy, Level::Hour1)
            .await
            .unwrap();

        print_report(month_count, "BTCUSDT", Level::Hour1, &result);
    }
}
```


## 🏆 Strategy Leaderboard

Same setup for every row: BTCUSDT futures, 1m source data, starting cash 1,000,000, position size
set by each strategy (mostly 0.01 BTC). The absolute numbers are therefore tiny — **read the
ordering, PF and drawdown, not a percentage return**. The 2026-10-28 archive hole is skipped.

Every window runs **the same code with parameters untouched**. 3 and 6 months are far too small to
mean anything; judge robustness from 12 months upwards.

### 3 months (2026-08 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 15 | 20% | 1.79 | **+405.78** | 233.51 | 1.74 |
| 2 | `strategy_volume_breakout` | 8 | 62.5% | 2.37 | +108.46 | 57.20 | 1.90 |
| 3 | `strategy_rsi_ema` | 16 | 43.8% | 1.49 | +74.27 | 138.88 | 0.53 |
| 4 | `strategy_fast` | 4 | 50% | 2.31 | +13.52 | 10.35 | 1.31 |
| 5 | `strategy_pullback_continuation` | 5 | 40% | 1.29 | +11.84 | 29.30 | 0.40 |
| 6 | `strategy_vegas_martingale` | 0 | 0% | — | 0.00 | 0.00 | — |
| 7 | `strategy_price_action` | 8 | 25% | 0.76 | -25.51 | 86.04 | -0.30 |
| 8 | `strategy_shadow_reversal` | 8 | 12.5% | 0.22 | -89.76 | 89.76 | -1.00 |
| 9 | `strategy_squeeze_dual_structure` | 14 | 14.3% | 0.41 | -108.57 | 139.42 | -0.78 |
| 10 | `strategy_dual_structure_trend` | 14 | 14.3% | 0.41 | -108.57 | 139.42 | -0.78 |
| 11 | `strategy_pullback_plus_breakout` | 14 | 14.3% | 0.41 | -108.57 | 139.42 | -0.78 |
| 12 | `strategy_pullback_trend` (baseline) | 11 | 9.1% | 0.24 | -114.93 | 114.93 | -1.00 |
| 13 | `strategy_fib_martingale` | 10 | 30% | 0.29 | -117.38 | 129.05 | -0.91 |
| 14 | `strategy_fib_martingale_050` | 14 | 42.9% | 0.32 | -136.26 | 162.67 | -0.84 |
| 15 | `strategy_fib_martingale_1h_050` | 63 | 44.4% | 0.65 | -141.22 | 155.79 | -0.91 |
| 16 | `strategy_fib_martingale_1h` | 61 | 31.1% | 0.55 | -175.26 | 175.26 | -1.00 |

### 6 months (2026-05 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 30 | 30% | 1.36 | **+309.43** | 233.51 | 1.33 |
| 2 | `strategy_volume_breakout` | 26 | 42.3% | 1.26 | +83.66 | 200.44 | 0.42 |
| 3 | `strategy_rsi_ema` | 37 | 35.1% | 1.09 | +34.27 | 195.81 | 0.18 |
| 4 | `strategy_fast` | 6 | 33.3% | 0.79 | -6.41 | 30.28 | -0.21 |
| 5 | `strategy_pullback_continuation` | 13 | 46.2% | 0.92 | -8.38 | 54.08 | -0.15 |
| 6 | `strategy_pullback_plus_breakout` | 24 | 33.3% | 0.9 | -26.45 | 139.42 | -0.19 |
| 7 | `strategy_dual_structure_trend` | 24 | 33.3% | 0.87 | -34.14 | 139.42 | -0.24 |
| 8 | `strategy_pullback_trend` (baseline) | 17 | 17.6% | 0.75 | -47.48 | 137.87 | -0.34 |
| 9 | `strategy_vegas_martingale` | 12 | 25% | 0.45 | -48.34 | 48.34 | -1.00 |
| 10 | `strategy_squeeze_dual_structure` | 24 | 33.3% | 0.77 | -63.74 | 139.42 | -0.46 |
| 11 | `strategy_fib_martingale` | 23 | 47.8% | 0.72 | -64.76 | 174.35 | -0.37 |
| 12 | `strategy_price_action` | 24 | 25% | 0.73 | -77.26 | 117.60 | -0.66 |
| 13 | `strategy_fib_martingale_050` | 27 | 51.9% | 0.52 | -148.03 | 227.59 | -0.65 |
| 14 | `strategy_shadow_reversal` | 18 | 11.1% | 0.31 | -180.00 | 180.00 | -1.00 |
| 15 | `strategy_fib_martingale_1h` | 105 | 38.1% | 0.65 | -227.78 | 316.21 | -0.72 |
| 16 | `strategy_fib_martingale_1h_050` | 113 | 46% | 0.61 | -290.49 | 369.26 | -0.79 |

### 12 months (2025-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_dual_structure_trend` | 72 | 44.4% | 1.5 | **+290.98** | 139.42 | 2.09 |
| 2 | `strategy_pullback_plus_breakout` | 72 | 43.1% | 1.48 | +269.81 | 139.42 | 1.94 |
| 3 | `strategy_pullback_continuation` | 34 | 55.9% | 2.35 | +264.37 | 54.08 | 4.89 |
| 4 | `strategy_squeeze_dual_structure` | 70 | 45.7% | 1.43 | +252.15 | 139.42 | 1.81 |
| 5 | `strategy_shadow_reversal` | 48 | 35.4% | 1.43 | +237.75 | 192.31 | 1.24 |
| 6 | `strategy_volume_breakout` | 43 | 44.2% | 1.26 | +175.88 | 203.38 | 0.86 |
| 7 | `strategy_rsi_ema` | 83 | 36.1% | 1.13 | +138.43 | 282.45 | 0.49 |
| 8 | `strategy_fib_martingale_050` | 59 | 59.3% | 1.16 | +93.95 | 227.59 | 0.41 |
| 9 | `strategy_pullback_trend` (baseline) | 51 | 35.3% | 1.13 | +57.93 | 137.87 | 0.42 |
| 10 | `strategy_horizon_long` | 65 | 30.8% | 1.03 | +56.64 | 527.07 | 0.11 |
| 11 | `strategy_price_action` | 62 | 32.3% | 1.06 | +48.83 | 227.77 | 0.21 |
| 12 | `strategy_vegas_martingale` | 16 | 37.5% | 1.16 | +18.82 | 39.80 | 0.47 |
| 13 | `strategy_fib_martingale` | 51 | 49% | 1.02 | +8.57 | 174.35 | 0.05 |
| 14 | `strategy_fast` | 45 | 28.9% | 0.86 | -30.56 | 81.68 | -0.37 |
| 15 | `strategy_fib_martingale_1h_050` | 228 | 50.9% | 0.92 | -120.90 | 384.13 | -0.31 |
| 16 | `strategy_fib_martingale_1h` | 205 | 41.5% | 0.8 | -281.74 | 420.85 | -0.67 |

### 24 months (2024-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 137 | 32.1% | 1.14 | **+694.60** | 1443.24 | 0.48 |
| 2 | `strategy_squeeze_dual_structure` | 151 | 49.7% | 1.35 | +520.11 | 143.62 | 3.62 |
| 3 | `strategy_rsi_ema` | 133 | 41.4% | 1.2 | +446.97 | 552.36 | 0.81 |
| 4 | `strategy_dual_structure_trend` | 145 | 47.6% | 1.31 | +428.09 | 222.27 | 1.93 |
| 5 | `strategy_volume_breakout` | 107 | 46.7% | 1.2 | +427.26 | 469.23 | 0.91 |
| 6 | `strategy_pullback_plus_breakout` | 140 | 46.4% | 1.27 | +368.90 | 226.36 | 1.63 |
| 7 | `strategy_pullback_continuation` | 70 | 57.1% | 1.64 | +333.83 | 72.11 | 4.63 |
| 8 | `strategy_price_action` | 128 | 37.5% | 1.17 | +324.21 | 311.34 | 1.04 |
| 9 | `strategy_shadow_reversal` | 110 | 31.8% | 1.19 | +288.99 | 291.36 | 0.99 |
| 10 | `strategy_pullback_trend` (baseline) | 100 | 40% | 1.24 | +259.44 | 242.99 | 1.07 |
| 11 | `strategy_fib_martingale` | 101 | 49.5% | 1.15 | +200.89 | 298.88 | 0.67 |
| 12 | `strategy_fib_martingale_1h_050` | 437 | 51.5% | 1.01 | +43.09 | 442.83 | 0.10 |
| 13 | `strategy_vegas_martingale` | 32 | 34.4% | 1 | -0.84 | 98.47 | -0.01 |
| 14 | `strategy_fast` | 156 | 27.6% | 0.84 | -128.71 | 181.64 | -0.71 |
| 15 | `strategy_fib_martingale_050` | 115 | 52.2% | 0.92 | -148.81 | 456.00 | -0.33 |
| 16 | `strategy_fib_martingale_1h` | 398 | 43.5% | 0.88 | -370.32 | 462.94 | -0.80 |

### 36 months (2023-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 190 | 33.7% | 1.21 | **+1426.04** | 1284.60 | 1.11 |
| 2 | `strategy_squeeze_dual_structure` | 245 | 49.4% | 1.35 | +770.79 | 143.62 | 5.37 |
| 3 | `strategy_dual_structure_trend` | 239 | 47.7% | 1.31 | +653.87 | 222.27 | 2.94 |
| 4 | `strategy_volume_breakout` | 168 | 48.2% | 1.2 | +622.18 | 469.23 | 1.33 |
| 5 | `strategy_pullback_plus_breakout` | 233 | 45.5% | 1.27 | +559.84 | 226.36 | 2.47 |
| 6 | `strategy_pullback_trend` (baseline) | 147 | 40.8% | 1.23 | +342.25 | 242.99 | 1.41 |
| 7 | `strategy_pullback_continuation` | 119 | 52.9% | 1.31 | +278.20 | 150.89 | 1.84 |
| 8 | `strategy_fib_martingale` | 143 | 51% | 1.15 | +272.65 | 298.88 | 0.91 |
| 9 | `strategy_price_action` | 195 | 37.4% | 1.1 | +264.46 | 311.34 | 0.85 |
| 10 | `strategy_rsi_ema` | 106 | 41.5% | 1.1 | +184.43 | 348.64 | 0.53 |
| 11 | `strategy_vegas_martingale` | 37 | 37.8% | 1.51 | +127.36 | 82.87 | 1.54 |
| 12 | `strategy_shadow_reversal` | 150 | 30% | 1.05 | +104.73 | 291.36 | 0.36 |
| 13 | `strategy_fib_martingale_050` | 166 | 53.6% | 0.92 | -205.49 | 488.56 | -0.42 |
| 14 | `strategy_fast` | 192 | 26% | 0.78 | -215.16 | 239.02 | -0.90 |
| 15 | `strategy_fib_martingale_1h_050` | 637 | 51.6% | 0.93 | -347.93 | 610.32 | -0.57 |
| 16 | `strategy_fib_martingale_1h` | 623 | 45.3% | 0.87 | -552.29 | 702.10 | -0.79 |

### 48 months (2022-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 263 | 31.9% | 1.22 | **+1724.22** | 1284.60 | 1.34 |
| 2 | `strategy_squeeze_dual_structure` | 327 | 47.7% | 1.37 | +892.28 | 143.62 | 6.21 |
| 3 | `strategy_dual_structure_trend` | 318 | 46.2% | 1.32 | +754.43 | 222.27 | 3.39 |
| 4 | `strategy_pullback_plus_breakout` | 312 | 44.2% | 1.28 | +656.30 | 226.36 | 2.90 |
| 5 | `strategy_volume_breakout` | 199 | 48.2% | 1.18 | +609.75 | 469.23 | 1.30 |
| 6 | `strategy_pullback_trend` (baseline) | 210 | 40% | 1.25 | +419.08 | 242.99 | 1.72 |
| 7 | `strategy_price_action` | 242 | 38.8% | 1.13 | +375.82 | 311.34 | 1.21 |
| 8 | `strategy_pullback_continuation` | 144 | 53.5% | 1.33 | +322.38 | 150.89 | 2.14 |
| 9 | `strategy_fib_martingale` | 193 | 48.2% | 1.11 | +240.38 | 298.88 | 0.80 |
| 10 | `strategy_vegas_martingale` | 51 | 41.2% | 1.56 | +154.67 | 82.87 | 1.87 |
| 11 | `strategy_rsi_ema` | 158 | 39.2% | 1.05 | +107.92 | 348.64 | 0.31 |
| 12 | `strategy_shadow_reversal` | 151 | 29.8% | 1.05 | +94.93 | 291.36 | 0.33 |
| 13 | `strategy_fast` | 194 | 25.8% | 0.76 | -248.63 | 272.49 | -0.91 |
| 14 | `strategy_fib_martingale_050` | 209 | 51.2% | 0.9 | -286.13 | 553.67 | -0.52 |
| 15 | `strategy_fib_martingale_1h_050` | 832 | 51.7% | 0.93 | -381.15 | 610.32 | -0.62 |
| 16 | `strategy_fib_martingale_1h` | 805 | 45.1% | 0.87 | -617.50 | 702.10 | -0.88 |

### 60 months (2021-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 330 | 31.2% | 1.1 | **+982.43** | 1284.60 | 0.76 |
| 2 | `strategy_volume_breakout` | 239 | 49% | 1.23 | +898.28 | 469.23 | 1.91 |
| 3 | `strategy_squeeze_dual_structure` | 406 | 47.3% | 1.28 | +810.78 | 199.98 | 4.05 |
| 4 | `strategy_dual_structure_trend` | 397 | 45.1% | 1.24 | +660.77 | 222.27 | 2.97 |
| 5 | `strategy_price_action` | 313 | 38.7% | 1.17 | +565.66 | 311.34 | 1.82 |
| 6 | `strategy_pullback_plus_breakout` | 391 | 42.7% | 1.2 | +541.44 | 226.36 | 2.39 |
| 7 | `strategy_rsi_ema` | 363 | 37.5% | 1.12 | +501.29 | 536.13 | 0.94 |
| 8 | `strategy_fib_martingale` | 235 | 48.9% | 1.13 | +328.98 | 298.88 | 1.10 |
| 9 | `strategy_pullback_trend` (baseline) | 261 | 37.5% | 1.15 | +300.54 | 242.99 | 1.24 |
| 10 | `strategy_pullback_continuation` | 184 | 52.2% | 1.22 | +256.47 | 150.89 | 1.70 |
| 11 | `strategy_vegas_martingale` | 69 | 34.8% | 1.28 | +101.73 | 84.83 | 1.20 |
| 12 | `strategy_shadow_reversal` | 174 | 30.5% | 1.03 | +79.01 | 291.36 | 0.27 |
| 13 | `strategy_fast` | 182 | 26.9% | 0.82 | -168.41 | 194.32 | -0.87 |
| 14 | `strategy_fib_martingale_050` | 259 | 52.1% | 0.94 | -176.81 | 670.84 | -0.26 |
| 15 | `strategy_fib_martingale_1h_050` | 1039 | 52.6% | 0.97 | -203.10 | 717.52 | -0.28 |
| 16 | `strategy_fib_martingale_1h` | 1039 | 46.1% | 0.89 | -594.61 | 702.10 | -0.85 |

### 72 months (2020-11 to 2026-10)

| # | Strategy | Trades | Win | PF | Net profit | Max DD | Profit/DD |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 398 | 31.2% | 1.26 | **+3017.09** | 1284.60 | 2.35 |
| 2 | `strategy_squeeze_dual_structure` | 488 | 48.2% | 1.34 | +1223.25 | 199.98 | 6.12 |
| 3 | `strategy_dual_structure_trend` | 474 | 46.2% | 1.29 | +1029.30 | 222.27 | 4.63 |
| 4 | `strategy_volume_breakout` | 294 | 49% | 1.19 | +959.34 | 469.23 | 2.04 |
| 5 | `strategy_pullback_plus_breakout` | 467 | 44.1% | 1.27 | +931.40 | 226.36 | 4.11 |
| 6 | `strategy_rsi_ema` | 420 | 38.3% | 1.18 | +903.11 | 514.77 | 1.75 |
| 7 | `strategy_price_action` | 379 | 38.8% | 1.16 | +680.64 | 311.34 | 2.19 |
| 8 | `strategy_pullback_continuation` | 230 | 53.9% | 1.35 | +546.27 | 205.99 | 2.65 |
| 9 | `strategy_fib_martingale` | 279 | 48.4% | 1.15 | +440.55 | 298.88 | 1.47 |
| 10 | `strategy_pullback_trend` (baseline) | 305 | 37% | 1.12 | +307.47 | 242.99 | 1.27 |
| 11 | `strategy_vegas_martingale` | 76 | 34.2% | 1.29 | +127.03 | 104.85 | 1.21 |
| 12 | `strategy_fib_martingale_050` | 303 | 52.5% | 0.95 | -183.62 | 670.84 | -0.27 |
| 13 | `strategy_shadow_reversal` | 228 | 28.9% | 0.9 | -311.41 | 637.37 | -0.49 |
| 14 | `strategy_fib_martingale_1h_050` | 1244 | 52.6% | 0.94 | -425.04 | 730.11 | -0.58 |
| 15 | `strategy_fib_martingale_1h` | 1203 | 46.6% | 0.93 | -453.10 | 781.00 | -0.58 |
| 16 | `strategy_fast` | 159 | 27% | 0.57 | -506.33 | 544.00 | -0.93 |



### Observations

1. **Short windows (3/6 months) carry almost no information**: at 3 months only 4 of 16 strategies are positive, at 6 months only 3. Yet `horizon_long` tops both (+405 / +309) — its profit is highly concentrated in the recent stretch, which is not the same as being reliable.
2. **`squeeze_dual_structure` is the steadiest over time**: positive in all six windows from 12 months up, profit/DD 1.81 / 3.62 / 5.37 / 6.21 / 4.05 / 6.12, with drawdown pinned between 143 and 200 — the only strategy whose risk does not grow as the window lengthens.
3. **`horizon_long` earns the most but is wildly unstable**: +3017.09 over 72 months (first place), yet only +982.43 over 60 months — less than the +1724.22 it makes over 48. The extra twelve months were a net loss, so it lives off a few trending stretches. Its 1284 drawdown is the largest in the table.
4. **`pullback_continuation` owns the short windows**: +264.37 at 12 months with a profit/DD of 4.89 (best in the table), dropping to 1.70 / 2.65 at 60/72 months — its edge is concentrated in the last two years.
5. **`fib_martingale` barely works even on 4h**: the 4h version makes +440.55 over 72 months; the 1h variants lose in every long window (PF 0.89–0.97) and the `_050` variant turns negative after 24 months. The more often they add, the more they lose.
6. **`fast` and `shadow_reversal` fall apart on long windows**: `fast` slides from −30 to −506 (PF 0.57), and `shadow_reversal` goes negative (−311.41) at 72 months. Their 12-month showing was regime luck.
