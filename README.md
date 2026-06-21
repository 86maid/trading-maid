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
- [🧠 Context](#-context)
- [📊 Series](#-series)
- [📈 Indicators](#-indicators)
- [🧩 Strategy as Struct](#-strategy-as-struct)
- [🛑 Error Handling](#-error-handling)
- [🚦 Hook Intercept](#-hook-intercept)
- [📊 Custom Series](#-custom-series)
- [🌐 Multi-Asset Strategy](#-multi-asset-strategy)
- [🧪 EMA Strategy Example](#-ema-strategy-example)

## ✨ Core Capabilities

- **Backtesting environment close to live trading**: simulates exchange matching logic, with built-in **slippage**, **leverage**, **margin**, and **forced liquidation** mechanics to reduce backtest/live deviation.
- **Live exchange abstraction**: provides a unified exchange interface for smooth migration from backtest to live trading.
- **Indicator and series tools**: includes common technical indicators and time-series processing utilities.
- **Backtest result visualization**: render candlesticks, orders, and position history in a web page.

## 🧭 Trading Model and Constraints

### 🧾 Order Types

- Supported: trigger price + (limit | market)
- Not supported: OCO (take-profit/stop-loss combo order)

### 📦 Position Type

- Margin mode: isolated
- Position direction: one-way
- Margin asset type: single-currency margin
- Margin management: dynamically adjusts position margin

### ⚙️ Order Processing Logic

- Margin freeze: market orders freeze margin when filled; reduce-only orders do not freeze margin.
- Matching timing: an order is placed on the current k-line and matched on the next k-line. a trigger order will be matched immediately on the current k-line once.
- Fill rules: market orders fill at `Open`; limit orders follow these rules:
    - Long
        - limit >= market: fill at worst price `High`
        - limit < market: fill at limit price
    - Short
        - limit <= market: fill at worst price `Low`
        - limit > market: fill at limit price
- Priority: when both pending-order conditions and liquidation conditions are met, pending orders are executed first.
- Fees: market orders use `taker_fee`; limit and liquidation orders use `maker_fee`.

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
            .sell_tp_sl("BTCUSDT", take_profit_price, stop_price, 0.01)
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

## 🧠 Context

In `Context`, `time`, `open`, `high`, `low`, and `close` are of type `&Series`, which is essentially a wrapped slice.

You can use `cx.close[0]` for the current k-line close, `cx.close[1]` for the previous k-line close, and so on.

You can use `&cx.close[2..]` to get a slice.

This type also overloads many operators, so you can omit index `[0]` in calculations, for example `cx.close + 100`.

You can destructure `Context` for easier OHLC usage.

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
    ma(cx.close, 30);
    ema(cx.close, 144);
    Ok(())
}
```

Calling `ema` directly may lead to slow backtests and incorrect calculations, because EMA depends on the previous EMA value. So it is recommended to use `EMACache` for fast and accurate calculation.

Use `EMACache::with_ema` to create an instance with an initial EMA value, then call `EMACache::update` on each k-line to update and get the current value:

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

The hook function is called after the strategy executes on each k-line.

```rust
async fn my_hook(_: KLine, exchange: Arc<dyn Exchange + 'static>) -> anyhow::Result<()> {
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
        if fr > dec!(0.0005) {
            return Ok(());
        }
    }

    Ok(())
}
```

If no series is registered for the given symbol/level/name, `cx[name]` returns an empty series (compare with `== []`).

## 🌐 Multi-Asset Strategy

Run a strategy across multiple symbols simultaneously. Pass an array of symbols to `run()` and use `cx.request()` to access other symbols' OHLCV data:

```rust
let exchange = LocalExchange::new([btc_data, eth_data]);

let mut engine = Engine::new(exchange, my_strategy);

// Pass multiple symbols to run()
engine.run(["BTCUSDT", "ETHUSDT"], Level::Hour1).await?;
```

In the strategy, access another symbol's context via `cx.request()`:

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // Access BTCUSDT directly (primary symbol)
    let ma_val = ma(cx.close, 30);

    // Access ETHUSDT via request
    if let Some(eth_cx) = cx.request("ETHUSDT", Level::Hour1) {
        // Use eth_cx like a normal Context
        let ma_val = ma(eth_cx.close, 30);
    }
    
    Ok(())
}
```

> ⚠️ **Note:** All symbols must share the same level, and their data time ranges must overlap — the strategy only fires when all symbols have data at the current bar.
>
> ⚠️ **Note:** The engine calls `next` concurrently for each symbol.

## 🧪 EMA Strategy Example

```rust
use std::sync::Arc;
use trading_maid::prelude::*;

struct MyStrategy {
    ema_cache144: EMACache,
    ema_cache169: EMACache,
    count: usize,
}

impl MyStrategy {
    pub fn new() -> Self {
        MyStrategy {
            ema_cache144: EMACache::with_ema(144, 80871),
            ema_cache169: EMACache::with_ema(169, 78705),
            count: 0,
        }
    }
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    // If the close stays below EMA for 50 consecutive k-lines and the current close breaks above EMA, open a short.
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
                .sell_tp_sl("BTCUSDT", cx.close - 1000, cx.close + 1000, 0.01)
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

// Stop backtesting when liquidation happens or an order is rejected (insufficient balance).
async fn my_hook(_: KLine, exchange: Arc<dyn Exchange + 'static>) -> anyhow::Result<()> {
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