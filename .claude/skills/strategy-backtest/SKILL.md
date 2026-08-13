---
name: strategy-backtest
description: AI 编写交易策略并通过回测迭代优化，直到策略满足合格标准（见 SKILL 的迭代终止条件）
---

# 策略编写与回测迭代

你是一个量化交易策略开发专家。你的任务是：编写交易策略 → 回测 → 分析结果 → 改进策略，反复迭代直到策略满足「迭代终止条件」中的全部合格标准。

## 项目背景

这是一个基于 Rust 的加密货币期货回测框架 `trading-maid`，支持：
- 币安数据下载与回测
- 保证金/杠杆/爆仓模拟
- 多级别 K 线（1m, 5m, 15m, 1h, 4h, 1d 等）
- 技术指标库与原始 OHLCV 数据（任意技术策略均可实现）

## 项目 API 速查

### 便捷回测（推荐，优先使用）

```rust
use trading_maid::prelude::*;

#[tokio::main]
async fn main() {
    // backtest(交易对, 下载月数, 策略, 策略K线级别)
    // 内部自动：下载1m数据 → DataSource → LocalExchange(cash=1000000, leverage=1) → Engine → run
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour1)
        .await
        .unwrap();

    // 打印汇总
    println!("summary: {:#?}", result.summarize());
}
```


## 技术指标

内置指标库提供常用技术指标（均线、RSI、MACD、布林带、ATR、摆动点等），完整签名见项目 README 的「Indicators」章节。**不要以为只能用这些指标**：任何技术策略思路都可以实现——可以自由组合任意指标，可以基于 `cx` 的原始 OHLCV 数据（`open/high/low/close/volume/time`）直接计算自实现指标，也可以使用 K 线形态、量价关系、支撑阻力、通道、统计方法等任何策略逻辑。框架不限制策略形式，只提供数据与下单能力。

## Context（策略上下文）可用字段

```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // --- OHLCV 数据（Series 类型，按索引访问，[0] = 当前值）---
    cx.open[0]    // 当前 K 线开盘价 (Decimal)
    cx.high[0]    // 当前 K 线最高价
    cx.low[0]     // 当前 K 线最低价
    cx.close[0]   // 当前 K 线收盘价
    cx.volume[0]  // 当前 K 线成交量
    cx.time[0]    // 当前 K 线时间戳 (u64)
    // 历史值: cx.close[1] 上一根, cx.close[2] 上上根 ...

    // --- 自定义系列（通过 engine.add_series 注册后可用）---
    cx["series_name"][0]

    // --- 查询操作 ---
    cx.get_position("BTCUSDT").await?              // → Result<Option<Position>>  None 表示空仓
    cx.get_pending_order_list("BTCUSDT").await?    // → Result<Vec<OrderMessage>>  当前挂单
    cx.cancel_all_order("BTCUSDT").await?          // → Result<()>

    // --- 下单（参数接受 &str / impl TryInto<Decimal>，返回订单 ID）---
    cx.buy("BTCUSDT", quantity).await?             // 市价买入 → Result<String>
    cx.sell("BTCUSDT", quantity).await?            // 市价卖出 → Result<String>
    cx.buy_limit("BTCUSDT", price, qty).await?     // 限价买入 → Result<String>
    cx.sell_limit("BTCUSDT", price, qty).await?    // 限价卖出 → Result<String>
    cx.buy_tp_sl("BTCUSDT", tp, sl, qty).await?    // 市价买入+止盈止损 → Result<(String, Result<String>, Result<String>)>
    cx.sell_tp_sl("BTCUSDT", tp, sl, qty).await?   // 市价卖出+止盈止损 → Result<(String, Result<String>, Result<String>)>
    cx.buy_trigger_limit("BTCUSDT", trigger, price, qty).await?   // 触发限价买单 → Result<String>
    cx.sell_trigger_limit("BTCUSDT", trigger, price, qty).await?  // 触发限价卖单 → Result<String>
}
```

## 工作流程

### 第一步：创建项目

用户在自己的工作目录创建一个新的 Cargo 项目，将 `trading-maid` 作为依赖：

```bash
cargo new my-strategy && cd my-strategy
```

`Cargo.toml` 中添加：

```toml
[dependencies]
trading-maid = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

### 第二步：编写策略

在 `src/main.rs` 中编写策略：

```rust
use trading_maid::prelude::*;

async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // 你的策略逻辑
    Ok(())
}

#[tokio::main]
async fn main() {
    let result = backtest("BTCUSDT", 12, my_strategy, Level::Hour1)
        .await
        .unwrap();

    println!("summary: {:#?}", result.summarize());
}
```

**策略编写原则：**
- 只做 BTCUSDT 单交易对
- 每次只持有一个方向的仓位（开仓前检查 `get_position` 是否为 `None`）
- 必须设置止盈止损（用 `sell_tp_sl` / `buy_tp_sl`）
- 综合多个信号源确认（技术指标、K 线形态、量价关系等均可），减少假突破
- 避免过于频繁的交易（手续费 taker 0.05% 会严重侵蚀利润）

### 第三步：编译运行

```bash
cargo run --release
```

### 第四步：分析结果

从输出中提取 `HistoryPositionSummary`，重点字段：

| 字段 | 含义 | 目标 |
|------|------|------|
| `total_profit` | 总利润（USDT） | **> 0**（核心指标） |
| `total_trades` | 总交易次数 | >= 回测月数 * 2（至少每月 2 单）|
| `win_rate` | 胜率 (0-1) | > 0.5 |
| `profit_loss_ratio` | 盈亏比 | > 1.0 |
| `total_fee` | 总手续费 | 相对利润尽量小 |
| `gross_profit` | 毛利润 | |
| `gross_loss_abs` | 毛亏损（绝对值）| |
| `best_trade` | 最佳单笔 | |
| `worst_trade` | 最差单笔 | |

> 表格中的目标数值即合格标准，完整定义见「迭代终止条件」。

### 第五步：迭代改进

根据 `HistoryPositionSummary` 中的 `total_profit`、`win_rate`、`total_trades`、`profit_loss_ratio` 等指标，分析策略的问题并自行判断改进方向，修改策略代码。

### 第六步：循环

重复步骤:

1. 分析上次回测结果的弱点
2. 修改 `src/main.rs` 中的策略代码
3. `cargo run --release 2>&1`
4. 对比新旧 summary，确认改进方向正确
5. 直到满足「迭代终止条件」中的全部合格标准

## 两种策略写法

### 写法 A：async fn（简单，推荐先用这个）
```rust
async fn my_strategy(cx: &Context<'_>) -> anyhow::Result<()> {
    // 直接写逻辑，每次调用重新计算指标
}
```

### 写法 B：struct + Strategy trait（有状态，需要跨 K 线保持状态时用）
```rust
struct MyStrategy {
    ema_fast: EMACache,
    ema_slow: EMACache,
    count: usize,
}

#[async_trait(?Send)]
impl Strategy for MyStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        let ema_f = self.ema_fast.update(cx.close);
        let ema_s = self.ema_slow.update(cx.close);
        // self.count 可以在多次调用间保持状态
    }
}
```
> `EMACache::new(n)` 需要 n 根 K 线预热（前期返回 None）。

## 注意事项

1. **只使用 `backtest()`**：统一用 `backtest` 进行配置，可以选择合适的 `Level`。
2. **数据下载**：首次运行需从币安下载历史数据（几分钟），后续使用 `~/.trading-maid/` 缓存。
3. **手续费**：`backtest()` 预设 maker 0.02%, taker 0.05%，高频交易手续费影响很大。
4. **Series 索引**：`series[0]` 是最新值，`series[1]` 是前一根。
5. **Rust edition 2024**：需要 Rust >= 1.85。跑不通先检查 `rustup update`。
6. 禁止使用可视化函数，例如 `open_in_server`, `open_in_browser`。
7. 必须使用 `--release` 提高回测速度。
8. 注意在调用 `buy_tp_sl/sell_tp_sl` (伪 oco 订单) 之前使用 `cancel_all_order` 取消所有订单。

## 迭代终止条件

策略合格标准：
- `total_profit > 0`（总利润为正）
- `win_rate > 0.5`（胜率至少 50%）
- `profit_loss_ratio > 1`（盈亏比大于 1）
- `total_trades >= 回测月数 * 2`（至少每月 2 单，即 `backtest()` 传入的 `month_count * 2`；如 12 个月回测需 >= 24 笔，保证足够样本量）
- 策略逻辑清晰、不严重过拟合

达到目标后，输出最终策略的完整代码和回测结果摘要。
