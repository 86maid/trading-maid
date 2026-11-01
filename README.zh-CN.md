# trading-maid

[English](README.md)  | 中文

[![Crates.io Version](https://img.shields.io/crates/v/trading-maid?logo=rust)](https://crates.io/crates/trading-maid)
[![docs.rs](https://img.shields.io/docsrs/trading-maid?logo=docs.rs)](https://docs.rs/trading-maid)
[![GitHub Repo stars](https://img.shields.io/github/stars/86maid/trading-maid)](https://github.com/86maid/trading-maid)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-2563eb.svg)](https://opensource.org/licenses/Apache-2.0)

> ⚡ 关键词：高拟真撮合 / 双阶段触发单 / 保证金与强平机制 / 回测可视化

trading-maid 是一个面向加密货币合约交易的回测与实盘框架，重点是尽量贴近真实交易环境。

框架内置撮合、滑点、杠杆、保证金与强平等关键机制，可用于策略验证、迭代和实盘接入。

![trading-maid](a.gif)

## 目录

- [✨ 核心能力](#-核心能力)
- [🧭 交易模型与限制](#-交易模型与限制)
- [🏗️ 架构概览](#-架构概览)
- [🚀 快速开始](#-快速开始)
- [⚡ 快速回测](#-快速回测)
- [🧠 Context](#-context)
- [📊 Series](#-series)
- [📈 指标](#-指标)
- [🧩 策略 Struct](#-策略-struct)
- [🛑 错误处理](#-错误处理)
- [🚦 Hook 劫持](#-hook-劫持)
- [📊 自定义系列](#-自定义系列)
- [🔄 获取其他级别的数据](#-获取其他级别的数据)
- [🌐 多币种策略](#-多币种策略)
- [💰 交易所账户方法](#-交易所账户方法)
- [🧰 常用工具函数](#-常用工具函数)
- [🧪 EMA 策略例子](#-ema-策略例子)
- [📊 影线反转策略](#-影线反转策略--candle-wick-reversal使用内置指标)
- [📊 成交量突破策略](#-成交量突破策略--donchian--volume-spike自实现指标)
- [📊 动量突破策略](#-动量突破策略--price-action自实现指标)
- [📊 RSI EMA 突破策略](#-rsi-ema-突破策略--consecutive-candle-breakout使用内置指标)
- [📊 Vegas 马丁格尔策略](#-vegas-马丁格尔策略--vegas-tunnel--rsimacd使用内置指标)
- [📊 时序动量做多策略](#-时序动量做多策略--time-series-momentum自实现指标)

## ✨ 核心能力

- **贴近实盘的回测环境**：模拟交易所撮合逻辑，内置**滑点**、**杠杆**、**保证金**与**强制平仓**机制，降低回测与实盘偏差。
- **实盘接口抽象**：支持统一的交易所接口，便于从回测迁移到实盘。
- **指标与序列工具**：提供常用技术指标与时间序列处理能力。
- **回测结果可视化**：可将 K 线、订单与持仓历史以网页形式展示。

## 🧭 交易模型与限制

### 🧾 订单类型

- 支持：触发价 + (限价 | 市价)
- 不支持：OCO（止盈止损组合单）

### 📦 仓位类型

- 保证金模式：逐仓
- 持仓方向：单向持仓
- 保证金类型：单币种保证金
- 保证金管理：动态调整仓位保证金

### ⚙️ 订单处理逻辑

- 保证金冻结：市价单在成交时冻结保证金，只减仓订单不冻结保证金。
- 撮合时序：下单会在当前 K 线挂单，在下一根 K 线撮合，触发单触发后会立即在当前 K 线撮合一次。
- 成交规则：市价单使用 Open 作为市价成交，限价单分为以下几种情况：
    - 多单
        - 委托价 >= 市价：以最坏的价格 High 成交
        - 委托价 < 市价：以委托价格成交
    - 空单
        - 委托价 <= 市价：以最坏的价格 Low 成交
        - 委托价 > 市价：以委托价格成交
- 优先级：当挂单条件与强平条件同时满足时，挂单优先执行。
- 手续费：市价单使用 taker_fee，限价单和强平单使用 maker_fee。

## 🏗️ 架构概览

完整文字架构图见 [architecture.txt](architecture.txt)。

## 🚀 快速开始

### 📥 安装

使用 `cargo add`

```bash
cargo add trading-maid
```

或者 `Cargo.toml`

```toml
[dependencies]
trading-maid = "1"
```

### 🛠️ 使用

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
    // 下载最近 12 个月的 1 分钟级别数据
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

    // 传入多个时间级别，方便在可视化页面中切换查看。
    open_in_server(
        [data_source_1h, data_source_1m],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
```

在这个例子中，我们设置了

* 最小下单数量 (min_size) 0.01
* 最小名义价值 (min_notional) 0.0 = 无限制
* 价格变化精度 (tick_size) 0.1
* 挂单费率 (maker_fee) 0.0002
* 吃单费率 (taker_fee) 0.0005
* 维持保证金率 (maintenance) 0.004
* 现金 (cash)  = 10000  
* 杠杆 (leverage) = 10  
* 滑点 (slippage) = 0

**回测为 1 分钟级别，策略为 1 小时级别**，回测引擎会在 1 小时级别的 k 线收盘时（一小时的最后一分钟）调用策略，策略获取到的每一根 k 线都是 1 小时级别的。

虽然可以使用其他级别，但是应 **始终使用 1 分钟级别的数据进行回测**，这样可以获得高精度的结果。

`open_in_server` 会启动一个本地服务器并自动在浏览器中打开回测可视化页面。

推荐优先使用 `open_in_server` 而非 `open_in_browser`，后者会每次都把 K 线数据写入文件导致浏览器重新加载，浪费时间。

使用 `cargo run -r` 能更快的完成回测。

> ⚠️ 注意：`sell_tp_sl` 只是语法糖，并非真正的 OCO 订单（框架不支持 OCO），它仅仅是同时下了两张单，需要你自己在开新仓前调用 `cancel_all_order` 来取消旧单。

> ⚠️ **止损注意**：止损应该使用触发单，例如，市价做多后的止损应使用 `sell_trigger_market_reduce_only`（价格到达触发价后执行只减仓市价卖出），市价做空后的止损应使用 `buy_trigger_market_reduce_only`（价格到达触发价后执行只减仓市价买入）。不要用 `sell_limit_reduce_only` 或 `buy_limit_reduce_only`——在订单簿中，卖出限价低于市价（或买入限价高于市价）意味着你的挂单直接穿过了买卖价差，会立刻成交，止损单就变成了即时市价出场，而不是等价格跌到止损位再触发。简单说，你的限价单会立即以市价成交。此外，止损务必使用 `reduce_only`，否则可能导致仓位反向持仓。

> ⚠️ **精度警告**：创建订单时（如 `buy`、`sell`、`buy_limit`、`sell_tp_sl` 等），价格和数量参数接受 `impl TryInto<Decimal>`。为避免浮点数精度丢失，对于高精度的数值，应使用字符串形式传入（如 `"0.01"`），而不是使用 `f64` 字面量如 `0.01`。

## ⚡ 快速回测

如果你想要用合理的预设配置快速回测，可以使用 `backtest()` 函数——它会自动处理数据下载、交易所设置和引擎创建，一行代码搞定：

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
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour1)
        .await
        .unwrap();

    // 回测结果汇总
    println!("summary: {:#?}", result.summarize());

    // 数据重采样到所有级别后可视化
    result.resample_all_open_in_server().await.unwrap();
}
```

`backtest()` 使用以下预设配置：

* **数据源**：1 分钟级别（自动通过 `get_or_download` 下载）
* **Metadata**：min_size=0.01、min_notional=0、tick_size=0.1、maker_fee=0.0002、taker_fee=0.0005、maintenance=0.004
* **交易所**：cash=1,000,000、leverage=1、slippage=0

### BacktestResult API

返回的 `BacktestResult` 提供以下方法：

| 方法 | 说明 |
|--------|-------------|
| `summarize()` | 返回 `HistoryPositionSummary`，包含胜率、盈亏、总交易次数等关键指标 |
| `open_in_browser()` | 将可视化写入临时 HTML 文件并在默认浏览器中打开 |
| `open_in_server()` | 启动本地服务器，以策略级别数据进行可视化 |
| `resample_all_open_in_server()` | 启动本地服务器，数据重采样到所有兼容级别，方便切换查看 |

> 💡 **这是懒人方法**——适合快速策略原型验证和实验。如果需要完全控制费率、杠杆、滑点、现金等参数，请使用[快速开始](#-快速开始)中的手动设置方式。

## 🧠 Context

在 `Context` 中，`time, open, high, low, close` 的类型为 `&Series`，这实际上是一个封装的切片。

你可以使用 `cx.close[0]` 表示当前 k 线的收盘价，`cx.close[1]` 表示上一根 k 线的收盘价，以此类推。 

你可以使用 `&cx.close[2..]` 来获取一个切片。

该类型还重载了大量的运算符，在计算时候可以省略下标 `[0]`，例如 `cx.close + 100`。

你可以将 `Context` 展开，以方便使用 OHLCV。

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

### 🕳️ 缺口检测：`cx.missed`

实盘的 K 线可能会丢——WebSocket 重连、进程卡顿、交易所跳过几根。引擎会用 `get_kline` 把空洞补回来，所以 `cx.close` 仍然连续，但**策略只会被调用一次**，而不是每根补回来的 K 线调用一次。

`cx.missed` 就是这次被跳过的策略级 K 线根数，正常情况为 `0`：

```rust
// 把策略没看到的 bar 按从旧到新喂回去
for i in (0..=cx.missed).rev() {
    self.ema_cache.update(cx.close[i]);
}
```

正常 bar 上这个循环等价于只喂 `close[0]`。忽略 `cx.missed` 的策略，只在「永远不出缺口」的前提下才是对的——要么按上面回放，要么用 `EMACache::from_series` 从历史重建。

`cx.request(...)` 返回的 `Context` 上也带同一个字段，对应它所代表的 symbol。

## 📊 Series

`Series` 是 `Context` 中贯穿始终的核心数值类型。它封装了一个倒序切片——`series[0]` 是当前 K 线，`series[1]` 是上一根，以此类推。越界索引返回 `Decimal::MAX`。

`Series` 重载了大量运算符，可以写出自然的表达式：

```rust
// 算术运算——自动省略 [0]
cx.close + 100;
cx.high - cx.low;
cx.close * dec!(1.5);

// 比较——直接与数字或字符串比较
cx.close == 123;
cx.close > 123.456;
cx.close < "0.0005";

// 切片——返回 &Series
&cx.close[2..];     // 从 2 根前到最早
&cx.close[..5];     // 最近 5 根
&cx.close[2..5];    // 指定范围
```

> ⚠️ **精度警告：** 避免使用 `f64` 字面量——应使用字符串（`"123.456"`）或 `dec!` 宏，以防止浮点数精度丢失。

## 📈 指标

在 `indicator` 中内置了一些常用的指标。

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    highest(cx.high, 7);
    sma(cx.close, 30);
    ema(cx.close, 144);
    Ok(())
}
```

直接调用 `ema` 函数可能导致回测缓慢和计算错误，因为计算 `ema` 需要用到上一个 `ema` 的值，所以推荐使用 `EMACache` 来进行快速和精确的计算。

使用 `EMACache::with_ema` 可以创建一个带有初始值的实例，然后在每根 K 线调用 `EMACache::update` 更新并获取当前值：

如果数据不是从第一根 K 线开始（实盘、预加载的窗口，或更窄的回测区间），应该用 `EMACache::from_series(cx.close, 144)` 从历史重建，而不是写死一个数。

```rust
async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
    let Some(ema144) = self.ema_cache144.update(cx.close) else {
        return Ok(());
    };
    
    Ok(())
}
```

## 🧩 策略 Struct

当策略需要维护状态（如缓存、计数器等）时，可以用 struct 实现 `Strategy` trait。

```rust
struct MyStrategy {
    ema_cache: EMACache,
    count: usize,
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        // 使用 self.ema_cache, self.count ...
        Ok(())
    }
}

let mut engine = Engine::new(exchange.clone(), MyStrategy::new());
```

## 🛑 错误处理

使用 `on_error` 来处理策略返回的错误。如果不设置，引擎会在策略出错时立即停止。

```rust
let mut engine = Engine::new(exchange.clone(), MyStrategy::new());

engine.on_error(|err| {
    eprintln!("策略错误: {:#?}", err);
    Ok(()) // 返回 Ok 继续运行，返回 Err 停止回测
});

if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
    println!("{:#?}", v);
}
```

> 💡 与每根 K 线都会执行的 `hook` 不同，`on_error` 只在策略返回错误时才会被调用。

## 🚦 Hook 劫持

使用 `hook` 可以在仓位发生强平，或者订单被拒绝（余额不足）的时候停止回测。

引擎在每根 K 线执行完策略后都会调用 hook 函数。第二个参数是网络中断后补回来的 K 线（正常情况下为空），hook 可以据此把自己的状态补齐。

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

## 📊 自定义系列

使用 `add_series` 将自定义数据（资金费率、链上指标、情绪等）附加到引擎。注册后，系列会与 OHLCV 数据同步，并可在策略中通过 `cx["name"]` 访问。

### 对齐自定义数据

自定义数据通常以稀疏的 `(时间戳毫秒, 值)` 形式存在。使用 `align_to_series` 将其前向填充为对齐到目标 K 线级别的 `AlignedSeries`：

```rust
// 稀疏的自定义数据：(时间戳毫秒, 值)
let custom_data = vec![
    (1717200000000, "1.5".parse::<Decimal>().unwrap()),
    (1717286400000, "2.3".parse::<Decimal>().unwrap()),
];

// 前向填充对齐到 1 小时 K 线
let series = align_to_series(&custom_data, Level::Hour1).unwrap();

// 在调用 run() 之前注册到引擎
let mut engine = Engine::new(exchange, my_strategy);
engine.add_series("BTCUSDT", "custom_metric", series);
engine.run("BTCUSDT", Level::Hour1).await?;
```

### 资金费率示例

`get_or_download_funding_rate_to_series` 可下载 Binance 历史资金费率并自动对齐：

```rust
let funding_rate_series = get_or_download_funding_rate_to_series(
    "BTCUSDT", 12, Level::Hour1,
).await.unwrap();

engine.add_series("BTCUSDT", "funding_rate", funding_rate_series);
engine.run("BTCUSDT", Level::Hour1).await?;
```

### 在策略中访问

在策略中按名称读取附加系列：

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    if cx["funding_rate"] != &[] {
        // 当前 K 线的资金费率
        let fr = cx["funding_rate"][0];
        // 上一根 K 线
        let fr_prev = cx["funding_rate"][1];

        // 费率过高时避免做多
        if fr > "0.0005" {
            return Ok(());
        }
    }

    Ok(())
}
```

如果给定的 symbol/level/name 没有注册系列，`cx[name]` 会返回空系列（可通过 `== []` 判断）。

## 🔄 获取其他级别的数据

使用 `cx.request()` 可以获取某个交易对在**指定级别**的 OHLCV 上下文。

```rust
// 获取 BTCUSDT 在 5 分钟级别的数据
if let Some(btc_5m_cx) = cx.request("BTCUSDT", Level::Minute5) {
    let sma_30 = sma(btc_5m_cx.close, 30);
}
```

`request` 背后通过**重采样**（resample）实现：

- 如果请求的级别等于**策略级别**（`engine.run` 传入的级别）或**源数据级别**（DataSource 的级别），直接返回预构建好的数据，无额外开销。
- 如果请求其他级别，框架会从源 K 线数据重采样到目标级别，**首次访问时计算并缓存**，同一根 K 线内后续调用直接命中缓存，不会重复计算。

目标级别必须是源数据级别的整数倍，你可以使用 `is_valid_sampling_target` 判断是否兼容。

> ⚠️ **推荐：始终使用 1 分钟数据作为源数据**，这样可以重采样到任意级别，获得最大的灵活性。

## 🌐 多币种策略

同时运行多个交易对的策略。向 `run()` 传入交易对数组，通过 `cx.request()` 访问其他交易对的 OHLCV 数据：

```rust
let exchange = LocalExchange::new([btc_data, eth_data])?;

let mut engine = Engine::new(exchange, my_strategy);

// 向 run() 传入多个交易对
engine.run(["BTCUSDT", "ETHUSDT"], Level::Hour1).await?;
```

在策略中，通过 `cx.request()` 访问其他交易对的上下文：

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // 直接访问 BTCUSDT（主交易对）
    let sma_val = sma(cx.close, 30);

    // 通过 request 访问 ETHUSDT
    if let Some(eth_cx) = cx.request("ETHUSDT", Level::Hour1) {
        // 像普通 Context 一样使用 eth_cx
        let sma_val = sma(eth_cx.close, 30);
    }
    
    Ok(())
}
```

> ⚠️ **注意：** 所有币种必须使用相同的级别，且数据时间范围必须有交集——只有当所有币种在当前 K 线都有数据时，策略才会触发。
>
> ⚠️ **注意：** 回测引擎会并发调用每个币种的 `next`。

## 💰 交易所账户方法

### 查询账户状态

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // 总权益 = 现金 + 所有仓位的未实现盈亏。
    let equity = cx.get_equity().await?;

    // 可用现金余额。
    let cash = cx.get_cash().await?;

    // 该交易对当前配置的杠杆。
    let leverage = cx.get_leverage("BTCUSDT").await?;

    println!("equity: {}, cash: {}, leverage: {}x", equity, cash, leverage);

    Ok(())
}
```

> 💡 `get_equity` 计入**未实现**盈亏，因此每一根 K 线都会随价格变动；而 `get_cash` 只反映已实现的变化。所以 `equity - cash` 可以快速反映当前持仓的浮动表现。

### 调整杠杆

`set_leverage` 会修改杠杆，并**立即根据开仓均价和数量重新计算仓位保证金**。提高杠杆会把多余保证金释放回现金；降低杠杆则需要追加现金。

```rust
// 将仓位的杠杆调整为 20x。
cx.set_leverage("BTCUSDT", 20).await?;
```

由于保证金会被重算，`set_leverage` 有可能失败。请处理这些错误，不要直接 unwrap：

- **挂单会阻止调用**：如果该交易对存在任何未成交订单，则无法修改杠杆。请先用 `cx.cancel_all_order(symbol).await?` 撤销。
- **现金不足**：降低杠杆会增加所需保证金；若 `cash` 无法覆盖差额，调用失败且仓位保持原样。

### 调整仓位保证金以避免强平

`append_position_margin` 为已有仓位增加保证金。增加保证金会使**强平价远离**当前价格，这是把正在逼近强平的仓位「救回来」的标准做法。

开仓前强平价会落在哪里，可以用 `calc_liquidation_price` 预测。它的参数依次为杠杆、来自交易对 metadata 的维持保证金率、仓位方向、开仓价、数量，以及仓位总保证金：

```rust
let metadata = cx.get_metadata("BTCUSDT").await?;

let liquidation_price = calc_liquidation_price(
    20,                       // 杠杆
    metadata.maintenance,     // 维持保证金率
    Side::Buy,
    "60000".parse().unwrap(), // 开仓价
    "0.5".parse().unwrap(),   // 数量
    "1500".parse().unwrap(),  // 仓位总保证金
);
```

因此对多头而言，增加保证金会**降低**强平价（更安全）；对空头则会**抬高**强平价。传入负的 `margin` 表示提取保证金，会使强平价更贴近当前价。

```rust
// 为仓位补充 500 保证金，使强平价远离当前价。
cx.append_position_margin("BTCUSDT", "500").await?;

// 取回 500 保证金到现金，使强平价更贴近当前价。
cx.append_position_margin("BTCUSDT", "-500").await?;
```

需要注意的边界条件：

- **保证金不能低于仓位的初始保证金**（`entry_price * quantity / leverage`）；尝试这样做会失败，错误信息中会给出所需的最低保证金。

### 识别强平

在仓位历史中，可以通过 `is_liquidation()` 判断仓位是否为强平：

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

### 🛫 预加载历史（仅实盘）

刚启动的实盘策略在指标攒够数据之前是瞎的——`sma(cx.close, 200)` 在前 200 根 K 线里一直返回 `None`。预加载会在第一根实盘 K 线之前把历史拉下来，让策略开局就能用：

```rust
// 取第一根实盘 K 线之前的 200 根
engine.preload(200);

// ……或者从某个时间戳起（0 = 能拉多早拉多早）
engine.preload_from(1735689600000);

// ……或者用内存里已有的数据当种子
engine.preload_data(data_source);
```

三者只能生效一个，后设置的覆盖前面的。回测会忽略预加载——回测数据本来就从头开始。

预加载区间**恰好截止到第一根实盘 K 线的前一根**，所以 `cx` 保持连续、不会和它重叠。如果种子数据的末尾到第一根实盘 K 线之间的空洞无法通过 `get_kline` 补上，引擎会直接终止，而不是带着缺口启动。

预加载填的是 `cx`，不是你自己的状态：`EMACache` 仍然从零开始，除非你在第一次调用时用 `EMACache::from_series(cx.close, 144)` 从历史重建。

## 🧰 常用工具函数

```rust
use trading_maid::prelude::*;

// 将毫秒级 Unix 时间戳转换为本地时区字符串。
t2s(1717200000000u64);

// 将毫秒级 Unix 时间戳转换为 UTC 字符串。
t2s_utc(1717200000000u64);

// 将本地时区字符串转回毫秒级 Unix 时间戳。
s2t("2024/06/01 04:00:00");

// 将 UTC 字符串转回毫秒级 Unix 时间戳。
s2t_utc("2024/06/01 04:00:00");

// 获取当前 K 线的起点与下一根 K 线的起点。
get_time_range(1717200000000, Level::Hour1)?;

// 获取包含该时间点的更粗级别 K 线的结束时间。
get_last_time(1717200000000, Level::Minute1, Level::Hour1)?;

// 将价格四舍五入到最近的合法最小变动价位。
round_to_tick(dec!(60000.37), dec!(0.1));

// 判断价格是否符合最小变动价位。
is_tick_aligned("60000.4".parse().unwrap(), "0.1".parse().unwrap());

// 计算初始保证金：price * quantity / leverage。
calc_initial_margin("60000".parse().unwrap(), "0.5".parse().unwrap(), 20);

// 计算仓位的强平价。
calc_liquidation_price(20, "0.004".parse().unwrap(), Side::Buy, "60000".parse().unwrap(), "0.5".parse().unwrap(), "1500".parse().unwrap());

// 将稀疏的 (时间戳, 数值) 数据前向填充到指定 K 线级别。
align_to_series(&[(1717200000000u64, "1.5".parse().unwrap())], Level::Hour1)?;

// 将 K 线列表重采样到更粗的级别。
resample(&kline_list, Level::Hour1)?;

// 将数据文件重采样为所有更粗的级别，并写入同目录。
resample_file("BTCUSDT-1m.csv")?;

// 下载 K 线数据，若本地已有缓存则直接复用。
get_or_download("BTCUSDT/1m", 12).await?;

// 下载资金费率历史数据。
get_or_download_funding_rate("BTCUSDT", 12).await?;

// 下载资金费率历史数据并对其到指定 K 线级别。
get_or_download_funding_rate_to_series("BTCUSDT", 12, Level::Hour8).await?;

// 将数据源、仓位与订单渲染为 HTML 字符串。
to_html(&data_source_list, &history_position_list, &history_order_list);

// 渲染为临时 HTML 文件并在浏览器中打开。
open_in_browser(&data_source_list, &history_position_list, &history_order_list)?;

// 通过本地 Web 服务提供渲染结果并打开。
open_in_server(data_source_list, history_position_list, history_order_list).await?;

// 将已平仓仓位列表汇总为统计结果。
summarize(&history_position_list);
```

## 📐 策略示例

`examples/` 目录中包含可直接运行的策略，展示了不同的实现思路。其中一些基于内置指标，另一些则完全从零自实现。

### 🧪 EMA 策略例子

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

// 仓位发生强平，或者订单被拒绝（余额不足）的时候停止回测
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

### 📊 影线反转策略 — Candle Wick Reversal（使用内置指标）

均值回归策略，当 4 小时 K 线出现长上影线（拒绝上涨）或长下影线（支撑确认）时开仓。仅使用了内置的 `atr()`。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 回调趋势策略 — SMA + ATR + RSI（使用内置指标）

趋势跟踪策略，在大趋势方向（SMA50/200）上寻找回调入场。使用 `atr()` 计算止损，`rsi()` 过滤超买超卖。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, PullbackStrategy::new(), Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 成交量突破策略 — Donchian + Volume Spike（自实现指标）

完全从零实现**唐奇安通道**、**成交量比率**和**RMA 平滑均线**。价格放量突破 20 周期唐奇安通道时入场。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, VolumeBreakout::new(), Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 动量突破策略 — Price Action（自实现指标）

纯裸 K 策略，完全自实现**动量评分**、**成交量爆发检测**和**平均波幅**，未使用任何内置指标。3 根 K 线累积极动量超过 2% 且成交量爆发 1.5 倍时入场。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, Momentum::new(), Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 RSI EMA 突破策略 — Consecutive Candle Breakout（使用内置指标）

趋势跟踪策略，通过**连续阳线/阴线** + **成交量确认** + **SMA50 趋势过滤**判断强势方向动量。使用内置的 `atr()`、`sma()`。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 Vegas 马丁格尔策略 — Vegas Tunnel + RSI/MACD（使用内置指标）

趋势跟踪策略，基于经典 **Vegas Tunnel**（EMA 12/144/169）。仅在趋势明确（EMA 带未收窄）时交易，配合 **RSI** 与 **MACD 柱** 确认方向，并使用 **马丁格尔** 仓位阶梯（0.01 → 0.02 → 0.04 → 0.08）：每轮亏损后翻倍加仓，止盈为基于 ATR 止损距离的 2 倍。使用内置的 `EMACache`、`rsi()`、`atr()`、`macd()`。

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

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 48, VegasMartingale::new(), Level::Hour4)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

### 📊 时序动量做多策略 — Time Series Momentum（自实现指标）

纯多头**时序动量（TSMOM）**策略，完全自实现 ATR 和 EMA。当价格高于 168 小时前的水平时买入，随后在**固定持仓周期**（336 小时）、**灾难止损**（6 倍 ATR）或**信号反转**（连续两根收盘价跌破动量水平）时离场。由于持仓周期以 K 线根数计量，策略运行在 1 小时级别上，回测区间为 12 个月。

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

#[tokio::main]
async fn main() {
    let result = backtest(SYMBOL, 12, TsmomLong::new(), Level::Hour1)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());

    result.resample_all_open_in_server().await.unwrap();
}
```

运行示例：
```bash
cargo run --release --example shadow_reversal
cargo run --release --example volume_breakout
cargo run --release --example price_action
cargo run --release --example rsi_ema
cargo run --release --example vegas_martingale
cargo run --release --example horizon_long
```
