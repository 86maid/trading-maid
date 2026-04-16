# trading-maid

[English](README.md)  | 中文

[![Crates.io Version](https://img.shields.io/crates/v/trading-maid?logo=rust)](https://crates.io/crates/trading-maid)
[![docs.rs](https://img.shields.io/docsrs/trading-maid?logo=docs.rs)](https://docs.rs/trading-maid)
[![GitHub Repo stars](https://img.shields.io/github/stars/86maid/trading-maid)](https://github.com/86maid/trading-maid)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-2563eb.svg)](https://opensource.org/licenses/Apache-2.0)

trading-maid 是一个面向加密货币合约交易的回测与实盘框架，重点是尽量贴近真实交易环境。
框架内置撮合、滑点、杠杆、保证金与强平等关键机制，可用于策略验证、迭代和实盘接入。

> ⚡ 关键词：高拟真撮合 / 双阶段触发单 / 保证金与强平机制 / 回测可视化

![trading-maid](a.png)

## 目录

- [✨ 核心能力](#-核心能力)
- [🧭 交易模型与限制](#-交易模型与限制)
- [🏗️ 架构概览](#-架构概览)
- [🚀 快速开始](#-快速开始)
- [🧠 Context](#-context)
- [📈 指标](#-指标)
- [🚦 Hook 劫持](#-hook-劫持)
- [🧪 一个完整的例子](#-一个完整的例子)

## ✨ 核心能力

- 贴近实盘的回测环境：模拟交易所撮合逻辑，降低回测与实盘偏差。
- 实盘接口抽象：支持统一的交易所接口，便于从回测迁移到实盘。
- 指标与序列工具：提供常用技术指标与时间序列处理能力。
- 回测结果可视化：可将 K 线、订单与持仓历史以网页形式展示。

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
trading-maid = "1.0.2"
```

### 🛠️ 使用

```rust
use trading_maid::prelude::*;

// 出现长上影线时开空单
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    let body = (cx.open - cx.close).abs();
    let line = (cx.high - cx.open).abs();
    let cond = cx.open > cx.close && line >= body * 2.0 && body >= 300.0;

    if cx.get_position("BTCUSDT").await?.is_none() && cond {
        println!("place order: {}", t2s(cx.time));

        let tp = cx.open - line;
        let sp = cx.open + line;

        cx.cancel_all_order("BTCUSDT").await?;
        cx.sell("BTCUSDT", 0.01).await?;
        cx.buy_limit_reduce_only("BTCUSDT", tp, 0.01).await?;
        cx.buy_trigger_market_reduce_only("BTCUSDT", sp, 0.01)
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
            min_size: 0.01,
            min_notional: 0.0,
            tick_size: 0.1,
            maker_fee: 0.0002,
            taker_fee: 0.0005,
            maintenance: 0.004,
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .cash(10000.0)
        .leverage(10)
        .slippage(0.0);

    let mut engine = Engine::new(exchange.clone(), my_strategy);

    // 使用 1 分钟级别数据进行回测，但在每个 1 小时级别的 K 线生成时都会调用策略函数
    if let Err(v) = engine.run("BTCUSDT", Level::Hour1).await {
        println!("{:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    // 从 1 分钟级别数据重采样得到 1 小时级别数据
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    open_in_browser(
        [data_source_1h, data_source_1m],
        history_position,
        history_order,
    )
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

回测为 1 分钟级别，策略为 1 小时级别，回测引擎会在 1 小时级别的 k 线收盘时（一小时的最后一分钟）调用策略，策略获取到的每一根 k 线都是 1 小时级别的。

虽然可以使用其他级别，但是应始终使用 1 分钟级别的数据进行回测，这样可以获得高精度的结果。

使用 `cargo run -r` 能更快的完成回测。

## 🧠 Context

在 `Context` 中，`time, open, high, low, close` 的类型为 `&Series`，这实际上是一个封装的切片。

你可以使用 `cx.close[0]` 表示当前 k 线的收盘价，`cx.close[1]` 表示上一根 k 线的收盘价，以此类推。 

你可以使用 `cx.close[2..]` 来获取一个切片。

该类型还重载了大量的运算符，在计算时候可以省略下标 `[0]`，例如 `cx.close + 100`。

你可以将 `Context` 展开，以方便使用 OHLC。

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
    }: &Context<'_>,
) -> anyhow::Result<()> {
    println!("time: {}", t2s(time));
    Ok(())
}
```

## 📈 指标

在 `indicator` 中内置了一些常用的指标。

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    highest(cx.high, 7);
    ma(cx.close, 30);
    ema(cx.close, 144);
    Ok(())
}
```

直接调用 `ema` 函数可能导致回测缓慢和计算错误，因为计算 `ema` 需要用到上一个 `ema` 的值，所以推荐使用 `EMACache` 来进行快速和精确的计算。

使用 `EMACache::with_ema` 可以创建一个带有初始值的实例。


## 🚦 Hook 劫持

使用 `hook` 可以在仓位发生强平，或者订单被拒绝（余额不足）的时候停止回测。

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

## 🧪 一个完整的例子

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
            ema_cache144: EMACache::with_ema(144, 80871.2),
            ema_cache169: EMACache::with_ema(169, 78705.2),
            count: 0,
        }
    }
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    // 如果连续 50 根 K 线收盘价都在 EMA 之下，且当前 K 线收盘价突破 EMA，则开空单
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let ema144 = self.ema_cache144.update(cx.close);
        let ema169 = self.ema_cache169.update(cx.close);

        if self.count >= 50
            && (cx.close >= ema144 || cx.close >= ema169)
            && cx.get_position("BTCUSDT").await?.is_none()
        {
            println!("place_order: {}", t2s(cx.time));

            cx.cancel_all_order("BTCUSDT").await?;
            cx.sell("BTCUSDT", 0.01).await?;
            cx.buy_limit_reduce_only("BTCUSDT", cx.close - 1000.0, 0.01)
                .await?;
            cx.buy_trigger_market_reduce_only("BTCUSDT", cx.close + 1000.0, 0.01)
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
            min_size: 0.01,
            min_notional: 0.0,
            tick_size: 0.1,
            maker_fee: 0.0002,
            taker_fee: 0.0005,
            maintenance: 0.004,
        },
    )
    .unwrap();

    let exchange = LocalExchange::new(data_source_1m.clone())
        .cash(10000.0)
        .leverage(10)
        .slippage(0.0);

    let mut engine = Engine::new(exchange.clone(), MyStrategy::new());

    engine.hook(my_hook);

    if let Err(v) = engine.run("BTCUSDT", Level::Minute5).await {
        println!("{:#?}", v);
    }

    let history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();
    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    let data_source_5m = data_source_1m.resample(Level::Minute5).unwrap();
    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();

    open_in_browser(
        [data_source_5m, data_source_1m, data_source_1h],
        history_position,
        history_order,
    )
    .unwrap();
}
```
