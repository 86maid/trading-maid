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
- 不支持：OCO（止盈止损组合单，币安也不支持）

### 📦 仓位类型

- 保证金模式：逐仓
- 持仓方向：单向持仓
- 保证金类型：单币种保证金
- 保证金管理：动态调整仓位保证金

### ⚙️ 订单处理逻辑

- **市价单**：以 `Open` 成交（套滑点后夹取到 `[Low, High]`）。
- **委托单**：
    - 买入：`Low <= 委托价` 即成交，成交价 `min(Open, 委托价)`
    - 卖出：`High >= 委托价` 即成交，成交价 `max(Open, 委托价)`
    - 即：开盘价已优于委托价 → 按开盘价成交（价格改善）；否则 → 按委托价成交。不吃滑点。
- **触发单**：触发价被触及后，在**同一根 K 线内**转成市价单或委托单，按上面两种规则成交。
    - 触发方向在挂单时定死：触发价在当前收盘价**上方** → `High >= 触发价` 才触发；**下方** → `Low <= 触发价` 才触发。
    - 方向与买卖无关：`sell_trigger_market(110.5)` 挂在市场价 105 上方，是“涨到 110.5 才卖”。
    - 转市价时成交价取 `max/min(Open, 触发价)`：跳空越过触发价 → 按开盘价；盘中穿越 → 按触发价。
- **时序**：下单当根不撮合，从下一根开始；只有触发单能在同一根内完成“触发 → 成交”。
- **保证金冻结**：市价单成交时冻结；委托单下单时按委托价冻结，成交价更优时退还差额；触发单在触发时才冻结；只减仓不冻结。
- **优先级**：同一根内按挂单顺序撮合，挂单先于强平。
- **手续费**：市价单、触发转市价用 `taker_fee`；委托单、触发转委托用 `maker_fee`；强平用 `taker_fee`。

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

K 线可能会丢——WebSocket 重连、进程卡顿，或者历史归档本身就没发布那一天。引擎会跳过空洞并通过 `cx.missed` 告诉策略，正常 bar 上是 `None`，否则说明是**哪一种**：

```rust
pub enum Missed {
    /// 这些 bar 有值，只是策略没被调用：`close[1]`、`close[2]`、…、`close[n]` 就是它们，
    /// 序列保持连续。
    Backfilled(usize),
    /// 这些 bar 不存在：`time` 会跳，`close[1..=n]` 取不到。
    Missing(usize),
}
```

两个变体里 `n` 的含义一致：本次调用覆盖了 `n + 1` 个策略周期。

**`Backfilled`**——实盘掉了几根，引擎通过 `get_kline` 补了回来。值就在序列里，有状态的策略可以回放，把指标追平：

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

**`Missing`**——这些 bar 在数据里根本不存在（历史归档的空洞）。没有东西可以回放：`close[1]` 是空洞之前那根，而越界索引会返回 `Decimal::MAX` 哨兵值，喂进任何指标都是灾难。唯一正确的反应是重置增量状态：

```rust
if let Some(Missed::Missing(_)) = cx.missed {
    self.ema_cache = EMACache::new(self.length);
}
```

忽略 `cx.missed` 的策略，只在它恒为 `None` 时才是对的——要么把两个变体都处理掉，要么用 `EMACache::from_series` 从完整历史重建。

另外两点：

- 只要一个策略级 K 线的源数据受过空洞影响，它就**整根丢弃**，绝不会带着残缺的 volume/high/low 交付出去——那种 bar 的时间戳看起来完全正常，策略根本发现不了。这也是 `Missing` 一定与 `cx.time` 的跳跃对应的原因。
- `cx.request(...)` 返回的 `Context` 上也带同一个字段，对应它所代表的 symbol。

真实归档里就有这种洞。例如 **Binance 合约 2026-10-28 一整天缺失**：月档 `BTCUSDT-1m-2026-10.zip`
只有 43200 行（31 天应为 44640 行），1h 周期同样少 24 根，日档 `2026-10-28.zip` 直接 404；
同一天**现货归档是完整的**。框架不做任何补齐——跑 12/24/36/48 个月的合约回测时，这一天会被
跳过，那天对应的策略级 K 线整根丢弃，下一次调用收到 `Missed::Missing(n)`。

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

目标级别的周期必须是源数据级别的整数倍，你可以使用 `is_valid_sampling_target` 判断是否兼容。
月线是例外：它按日历分月、长度不固定（28~31 天），所以只允许从**周期能整除一天**的级别聚合
（1m～12h、1d）。从 3d 或 1w 聚合月线会把跨月的那根 K 线整根算进上个月，因此会被拒绝。

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

`examples/` 下的 `strategy_*` 是完整策略：进场/出场规则、仓位管理、完整回测。完整源码全部收录在下面，按 72 个月净利润排序。

`demo_*` 是最小示例（`demo_minimal` 只下一笔单、`demo_indicator` 演示内置指标 + hook、`demo_funding_rate` 演示资金费率序列），源码见 `examples/`。

| # | 文件 | 周期 | 思路 |
|--:|---|:--:|---|
| 1 | `strategy_horizon_long` | 1h | 时序动量做多（TSMOM），只做多 |
| 2 | `strategy_squeeze_dual_structure` | 4h | 双结构 + 布林收口突破，负责「趋势还没形成时」的入场 |
| 3 | `strategy_dual_structure_trend` | 4h | 双结构趋势（风控归一版）：结构 A 止损最小 0.5 ATR，结构 B 移动止损 min(1 ATR, 0.5R) |
| 4 | `strategy_volume_breakout` | 4h | Donchian 通道突破 + 成交量放大确认 |
| 5 | `strategy_pullback_plus_breakout` | 4h | 双结构：回调接续 + 动量延续，两套入场共用一份风控 |
| 6 | `strategy_rsi_ema` | 4h | 连续 K 线突破 + RSI/EMA 过滤 |
| 7 | `strategy_price_action` | 4h | 动量突破，指标从零自实现 |
| 8 | `strategy_pullback_continuation` | 4h | EMA 三线排列 + ADX 强度 + 突破前高入场 + ATR 移动止损 |
| 9 | `strategy_fib_martingale` | 4h | 斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（0.382） |
| 10 | `strategy_pullback_trend` | 4h | SMA50/200 定趋势 + RSI 回调进场（**基准**） |
| 11 | `strategy_vegas_martingale` | 4h | Vegas 隧道 + RSI/MACD + 马丁格尔加仓 |
| 12 | `strategy_fib_martingale_050` | 4h | 同上，0.5 回撤入场 |
| 13 | `strategy_shadow_reversal` | 4h | 长影线反转，ATR 定止盈止损 |
| 14 | `strategy_fib_martingale_1h_050` | 1h | 同上，1h + 0.5 回撤入场 |
| 15 | `strategy_fib_martingale_1h` | 1h | 同上，周期换成 1h |
| 16 | `strategy_fast` | 1h | 长上影线开空，规则最简单的一个 |

### 1. `strategy_horizon_long`

时序动量做多（TSMOM），只做多。

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

双结构 + 布林收口突破，负责「趋势还没形成时」的入场。

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

双结构趋势（风控归一版）：结构 A 止损最小 0.5 ATR，结构 B 移动止损 min(1 ATR, 0.5R)。

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

Donchian 通道突破 + 成交量放大确认。

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

双结构：回调接续 + 动量延续，两套入场共用一份风控。

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

连续 K 线突破 + RSI/EMA 过滤。

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

动量突破，指标从零自实现。

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

EMA 三线排列 + ADX 强度 + 突破前高入场 + ATR 移动止损。

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

斐波那契回撤 + 拐点流动性 + 真空区 + 解套加仓（0.382）。

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

SMA50/200 定趋势 + RSI 回调进场（**基准**）。

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

Vegas 隧道 + RSI/MACD + 马丁格尔加仓。

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

同上，0.5 回撤入场。

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

长影线反转，ATR 定止盈止损。

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

同上，1h + 0.5 回撤入场。

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

同上，周期换成 1h。

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

长上影线开空，规则最简单的一个。

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


## 🏆 策略收益排行

口径统一：BTCUSDT 合约 1m 源数据，初始资金 1,000,000，仓位由各策略自行设定（多数是 0.01 BTC），
所以绝对数字很小，**看相对排名、PF 和回撤，不要看收益率百分比**。2026-10-28 那个归档空洞被跳过。

每个窗口都是**同一份代码、参数未按窗口调整**。3/6 个月样本太小，只作参考；判断稳健性看 12 个月以上。

### 3 个月（2026-08 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
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
| 12 | `strategy_pullback_trend`（基准） | 11 | 9.1% | 0.24 | -114.93 | 114.93 | -1.00 |
| 13 | `strategy_fib_martingale` | 10 | 30% | 0.29 | -117.38 | 129.05 | -0.91 |
| 14 | `strategy_fib_martingale_050` | 14 | 42.9% | 0.32 | -136.26 | 162.67 | -0.84 |
| 15 | `strategy_fib_martingale_1h_050` | 63 | 44.4% | 0.65 | -141.22 | 155.79 | -0.91 |
| 16 | `strategy_fib_martingale_1h` | 61 | 31.1% | 0.55 | -175.26 | 175.26 | -1.00 |

### 6 个月（2026-05 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 30 | 30% | 1.36 | **+309.43** | 233.51 | 1.33 |
| 2 | `strategy_volume_breakout` | 26 | 42.3% | 1.26 | +83.66 | 200.44 | 0.42 |
| 3 | `strategy_rsi_ema` | 37 | 35.1% | 1.09 | +34.27 | 195.81 | 0.18 |
| 4 | `strategy_fast` | 6 | 33.3% | 0.79 | -6.41 | 30.28 | -0.21 |
| 5 | `strategy_pullback_continuation` | 13 | 46.2% | 0.92 | -8.38 | 54.08 | -0.15 |
| 6 | `strategy_pullback_plus_breakout` | 24 | 33.3% | 0.9 | -26.45 | 139.42 | -0.19 |
| 7 | `strategy_dual_structure_trend` | 24 | 33.3% | 0.87 | -34.14 | 139.42 | -0.24 |
| 8 | `strategy_pullback_trend`（基准） | 17 | 17.6% | 0.75 | -47.48 | 137.87 | -0.34 |
| 9 | `strategy_vegas_martingale` | 12 | 25% | 0.45 | -48.34 | 48.34 | -1.00 |
| 10 | `strategy_squeeze_dual_structure` | 24 | 33.3% | 0.77 | -63.74 | 139.42 | -0.46 |
| 11 | `strategy_fib_martingale` | 23 | 47.8% | 0.72 | -64.76 | 174.35 | -0.37 |
| 12 | `strategy_price_action` | 24 | 25% | 0.73 | -77.26 | 117.60 | -0.66 |
| 13 | `strategy_fib_martingale_050` | 27 | 51.9% | 0.52 | -148.03 | 227.59 | -0.65 |
| 14 | `strategy_shadow_reversal` | 18 | 11.1% | 0.31 | -180.00 | 180.00 | -1.00 |
| 15 | `strategy_fib_martingale_1h` | 105 | 38.1% | 0.65 | -227.78 | 316.21 | -0.72 |
| 16 | `strategy_fib_martingale_1h_050` | 113 | 46% | 0.61 | -290.49 | 369.26 | -0.79 |

### 12 个月（2025-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_dual_structure_trend` | 72 | 44.4% | 1.5 | **+290.98** | 139.42 | 2.09 |
| 2 | `strategy_pullback_plus_breakout` | 72 | 43.1% | 1.48 | +269.81 | 139.42 | 1.94 |
| 3 | `strategy_pullback_continuation` | 34 | 55.9% | 2.35 | +264.37 | 54.08 | 4.89 |
| 4 | `strategy_squeeze_dual_structure` | 70 | 45.7% | 1.43 | +252.15 | 139.42 | 1.81 |
| 5 | `strategy_shadow_reversal` | 48 | 35.4% | 1.43 | +237.75 | 192.31 | 1.24 |
| 6 | `strategy_volume_breakout` | 43 | 44.2% | 1.26 | +175.88 | 203.38 | 0.86 |
| 7 | `strategy_rsi_ema` | 83 | 36.1% | 1.13 | +138.43 | 282.45 | 0.49 |
| 8 | `strategy_fib_martingale_050` | 59 | 59.3% | 1.16 | +93.95 | 227.59 | 0.41 |
| 9 | `strategy_pullback_trend`（基准） | 51 | 35.3% | 1.13 | +57.93 | 137.87 | 0.42 |
| 10 | `strategy_horizon_long` | 65 | 30.8% | 1.03 | +56.64 | 527.07 | 0.11 |
| 11 | `strategy_price_action` | 62 | 32.3% | 1.06 | +48.83 | 227.77 | 0.21 |
| 12 | `strategy_vegas_martingale` | 16 | 37.5% | 1.16 | +18.82 | 39.80 | 0.47 |
| 13 | `strategy_fib_martingale` | 51 | 49% | 1.02 | +8.57 | 174.35 | 0.05 |
| 14 | `strategy_fast` | 45 | 28.9% | 0.86 | -30.56 | 81.68 | -0.37 |
| 15 | `strategy_fib_martingale_1h_050` | 228 | 50.9% | 0.92 | -120.90 | 384.13 | -0.31 |
| 16 | `strategy_fib_martingale_1h` | 205 | 41.5% | 0.8 | -281.74 | 420.85 | -0.67 |

### 24 个月（2024-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
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
| 10 | `strategy_pullback_trend`（基准） | 100 | 40% | 1.24 | +259.44 | 242.99 | 1.07 |
| 11 | `strategy_fib_martingale` | 101 | 49.5% | 1.15 | +200.89 | 298.88 | 0.67 |
| 12 | `strategy_fib_martingale_1h_050` | 437 | 51.5% | 1.01 | +43.09 | 442.83 | 0.10 |
| 13 | `strategy_vegas_martingale` | 32 | 34.4% | 1 | -0.84 | 98.47 | -0.01 |
| 14 | `strategy_fast` | 156 | 27.6% | 0.84 | -128.71 | 181.64 | -0.71 |
| 15 | `strategy_fib_martingale_050` | 115 | 52.2% | 0.92 | -148.81 | 456.00 | -0.33 |
| 16 | `strategy_fib_martingale_1h` | 398 | 43.5% | 0.88 | -370.32 | 462.94 | -0.80 |

### 36 个月（2023-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 190 | 33.7% | 1.21 | **+1426.04** | 1284.60 | 1.11 |
| 2 | `strategy_squeeze_dual_structure` | 245 | 49.4% | 1.35 | +770.79 | 143.62 | 5.37 |
| 3 | `strategy_dual_structure_trend` | 239 | 47.7% | 1.31 | +653.87 | 222.27 | 2.94 |
| 4 | `strategy_volume_breakout` | 168 | 48.2% | 1.2 | +622.18 | 469.23 | 1.33 |
| 5 | `strategy_pullback_plus_breakout` | 233 | 45.5% | 1.27 | +559.84 | 226.36 | 2.47 |
| 6 | `strategy_pullback_trend`（基准） | 147 | 40.8% | 1.23 | +342.25 | 242.99 | 1.41 |
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

### 48 个月（2022-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 263 | 31.9% | 1.22 | **+1724.22** | 1284.60 | 1.34 |
| 2 | `strategy_squeeze_dual_structure` | 327 | 47.7% | 1.37 | +892.28 | 143.62 | 6.21 |
| 3 | `strategy_dual_structure_trend` | 318 | 46.2% | 1.32 | +754.43 | 222.27 | 3.39 |
| 4 | `strategy_pullback_plus_breakout` | 312 | 44.2% | 1.28 | +656.30 | 226.36 | 2.90 |
| 5 | `strategy_volume_breakout` | 199 | 48.2% | 1.18 | +609.75 | 469.23 | 1.30 |
| 6 | `strategy_pullback_trend`（基准） | 210 | 40% | 1.25 | +419.08 | 242.99 | 1.72 |
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

### 60 个月（2021-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
|--:|---|--:|--:|--:|--:|--:|--:|
| 1 | `strategy_horizon_long` | 330 | 31.2% | 1.1 | **+982.43** | 1284.60 | 0.76 |
| 2 | `strategy_volume_breakout` | 239 | 49% | 1.23 | +898.28 | 469.23 | 1.91 |
| 3 | `strategy_squeeze_dual_structure` | 406 | 47.3% | 1.28 | +810.78 | 199.98 | 4.05 |
| 4 | `strategy_dual_structure_trend` | 397 | 45.1% | 1.24 | +660.77 | 222.27 | 2.97 |
| 5 | `strategy_price_action` | 313 | 38.7% | 1.17 | +565.66 | 311.34 | 1.82 |
| 6 | `strategy_pullback_plus_breakout` | 391 | 42.7% | 1.2 | +541.44 | 226.36 | 2.39 |
| 7 | `strategy_rsi_ema` | 363 | 37.5% | 1.12 | +501.29 | 536.13 | 0.94 |
| 8 | `strategy_fib_martingale` | 235 | 48.9% | 1.13 | +328.98 | 298.88 | 1.10 |
| 9 | `strategy_pullback_trend`（基准） | 261 | 37.5% | 1.15 | +300.54 | 242.99 | 1.24 |
| 10 | `strategy_pullback_continuation` | 184 | 52.2% | 1.22 | +256.47 | 150.89 | 1.70 |
| 11 | `strategy_vegas_martingale` | 69 | 34.8% | 1.28 | +101.73 | 84.83 | 1.20 |
| 12 | `strategy_shadow_reversal` | 174 | 30.5% | 1.03 | +79.01 | 291.36 | 0.27 |
| 13 | `strategy_fast` | 182 | 26.9% | 0.82 | -168.41 | 194.32 | -0.87 |
| 14 | `strategy_fib_martingale_050` | 259 | 52.1% | 0.94 | -176.81 | 670.84 | -0.26 |
| 15 | `strategy_fib_martingale_1h_050` | 1039 | 52.6% | 0.97 | -203.10 | 717.52 | -0.28 |
| 16 | `strategy_fib_martingale_1h` | 1039 | 46.1% | 0.89 | -594.61 | 702.10 | -0.85 |

### 72 个月（2020-11 ~ 2026-10）

| # | 策略 | 交易 | 胜率 | PF | 净利润 | 最大回撤 | 利润/回撤 |
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
| 10 | `strategy_pullback_trend`（基准） | 305 | 37% | 1.12 | +307.47 | 242.99 | 1.27 |
| 11 | `strategy_vegas_martingale` | 76 | 34.2% | 1.29 | +127.03 | 104.85 | 1.21 |
| 12 | `strategy_fib_martingale_050` | 303 | 52.5% | 0.95 | -183.62 | 670.84 | -0.27 |
| 13 | `strategy_shadow_reversal` | 228 | 28.9% | 0.9 | -311.41 | 637.37 | -0.49 |
| 14 | `strategy_fib_martingale_1h_050` | 1244 | 52.6% | 0.94 | -425.04 | 730.11 | -0.58 |
| 15 | `strategy_fib_martingale_1h` | 1203 | 46.6% | 0.93 | -453.10 | 781.00 | -0.58 |
| 16 | `strategy_fast` | 159 | 27% | 0.57 | -506.33 | 544.00 | -0.93 |



### 观察

1. **短窗口（3/6 个月）几乎没有信息量**：3 个月里 16 个策略只有 4 个为正，6 个月里只剩 3 个。`horizon_long` 却在这两个窗口都是第一（+405 / +309），说明它的收益高度集中在最近一段——不是"稳定赚钱"。
2. **`squeeze_dual_structure` 长期最稳**：12/24/36/48/60/72 六个窗口全部为正，利润/回撤 1.81 / 3.62 / 5.37 / 6.21 / 4.05 / 6.12，回撤始终压在 143～200，是唯一一个"窗口拉长、风险不放大"的策略。
3. **`horizon_long` 绝对收益最高但极不稳定**：72 个月 +3017.09（第一），但 60 个月只有 +982.43——比 48 个月的 +1724.22 还少。多出来的那 12 个月是净亏的，说明它靠少数几段趋势行情吃饭。回撤常年 1284，是所有策略里最大。
4. **`pullback_continuation` 是短窗口之王**：12 个月 +264.37、利润/回撤 4.89（全表最高），但 60/72 个月掉到 1.70 / 2.65，优势集中在近两年。
5. **`fib_martingale` 只有 4h 版本勉强能用**：4h 版 72 个月 +440.55；1h 版四个长窗口全负（PF 0.89～0.97），`_050` 变体在 24 个月之后也转负。加仓频率越高亏得越多。
6. **`fast` / `shadow_reversal` 长窗口失效**：`fast` 从 −30 一路走到 −506（PF 0.57），`shadow_reversal` 在 72 个月转负 −311.41。12 个月上的表现属于区间红利。
