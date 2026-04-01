---
name: crypto-backtest
description: "加密货币回测器，当用户希望评估加密交易策略效果并进行回测与结果查看时使用。"
---

# crypto-backtest

## 目标

用于在本仓库中完成“加密策略回测 -> 结果解读 -> 风险复盘 -> 可视化展示”的完整闭环。

默认偏保守：优先降低噪声信号与过度标注风险，避免给出看起来漂亮但不可复现的结论。

## 何时使用

当用户出现以下意图时应触发：

- 询问策略是否有效、是否能盈利、胜率如何
- 要求做 BTC/ETH（或其他币对）的回测
- 要求看订单历史、持仓历史、收益分解
- 要求参数优化、结果对比、可视化复盘
- 讨论现货/期货/永续在回测中的差异

## 项目导入与使用

### 1. 先新建 Rust 项目（外部项目场景）

在外部项目中使用时，第一步必须先创建工程：

```bash
cargo new my-backtest
cd my-backtest
```

若需要异步运行策略，建议使用 tokio 运行时。

### 2. 在外部 Rust 项目导入

适用于在你自己的策略工程中复用 trading-maid。

使用 crates.io 版本。

```bash
cargo add trading-maid
```

或在 Cargo.toml 中添加：

```toml
[dependencies]
trading-maid = "1.0.0"
```

最小使用流程：

1. `use trading_maid::prelude::*;`
2. 准备数据：`get_or_download` 或 `DataSource::from_file_metadata`
3. 创建交易所：`LocalExchange::new(...).cash(...).leverage(...).slippage(...)`
4. 创建引擎：`Engine::new(exchange.clone(), my_strategy)`
5. 执行回测：`engine.run(symbol, level).await`
6. 结果输出：`summarize(...)` 或 `open_in_browser(...)`

## 执行流程

### 1. 明确回测边界

在动手前先锁定以下信息，缺失则先补齐：

- 标的与市场：如 BTCUSDT 永续
- 数据周期：回测底层建议固定 1m
- 策略周期：如 1h、4h
- 手续费与滑点：maker_fee、taker_fee、slippage
- 杠杆与保证金：cash、leverage、maintenance
- 样本区间：训练/验证/测试（至少给出时间范围）

若用户未指定，按仓库示例的保守默认值执行，并在结果中明确写出默认假设。

### 2. 优先复用仓库现有能力

本仓库的关键能力与入口：

- 回测主流程：Engine + Strategy + LocalExchange
- 统计汇总：summarize
- 可视化导出：open_in_browser / to_html
- 参考示例：examples/simple.rs

优先在现有示例上最小改动，不重复造轮子。

### 3. 结果输出（固定结构）

回测结果展示必须二选一：使用 #sym:summarize 输出统计结果，或者使用浏览器可视化（open_in_browser / to_html）。

每次回测结果按以下结构输出，避免只给单一收益数字：

1. 回测配置
2. 核心指标
3. 订单与持仓摘要
4. 风险与稳定性
5. 结论与下一步

建议字段：

- 回测配置：symbol、数据级别、策略级别、时间区间、fee、slippage、leverage
- 核心指标：total_trades、win_rate、total_profit、avg_profit、best_trade、worst_trade、total_fee、profit_loss_ratio
- 风险项：是否出现强平、是否出现拒单、单笔最大回撤近似、连续亏损次数

### 4. 参数优化原则

参数优化必须遵守：

- 先定义目标函数（如收益风险比或净利/回撤）
- 使用时间切分，避免把同一时段反复拟合
- 报告中同时给出“最优参数”和“次优稳健区间”
- 不只展示最好结果，至少给出失败样本或退化区间

## 策略编写

### 1. 策略设计最小清单

在开始写策略代码前，先写清楚以下 5 项：

- 入场条件：必须可量化，避免主观描述
- 出场条件：止盈、止损、失效退出三类至少覆盖两类
- 仓位规则：每笔数量、是否允许加仓、是否允许反手
- 风险约束：单笔风险上限、连续亏损熔断、最大持仓时长
- 交易频率约束：避免过密交易导致手续费吞噬收益

若用户未给出完整条件，优先采用“低频、强确认、少噪声”策略版本。

### 2. 信号规则（严格模式）

为了减少乱标注和噪声信号，默认采用严格信号：

- 多条件共振：至少 2 到 3 个独立条件同时满足再入场
- 过滤震荡：波动不足或趋势不明确时不交易
- 禁止追涨杀跌：单根极端 K 线触发时要求二次确认
- 冷却窗口：平仓后 N 根 K 线内不重复开同向仓

当用户要求“信号更积极”时，再逐步放宽条件，并在结果里说明放宽项。

### 3. 防前视偏差与执行偏差

策略实现必须避免以下错误：

- 使用未来数据：只使用当前及历史数据，不使用未收盘信息推导成交结果
- 忽略撮合时序：下单与成交并非同一时刻
- 忽略交易成本：fee、slippage 进入全部评估指标
- 忽略交易约束：tick_size、min_size、min_notional 必须校验

### 4. 策略结果解释要求

当用户要求“写策略并给结论”时，输出至少包含：

1. 策略逻辑摘要（入场、出场、风控）
2. 与基线策略对比（如 buy-and-hold 或无过滤版本）
3. 失效场景（震荡、单边极端、手续费升高）
4. 可执行的下一步（先调哪个参数，预期影响什么指标）

### 5. README 完整例子

以下示例来自 README，可直接作为完整策略模板使用：

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

## 真实性约束

输出结论时必须提醒以下交易仿真约束（若涉及）：

- 建议始终使用 1m 数据作为回测底层，以提高成交近似精度
- 下单与撮合存在时序，避免将信号与成交视为同一时刻
- 手续费、滑点、最小下单单位、最小名义价值必须进入计算
- 回测高收益若来自低交易次数，应标记统计不稳定风险

## 文风与结论要求

- 先给事实，再给结论，最后给改进建议
- 对不确定结论使用条件化表述，不夸大收益
- 默认采用“保守解读”：优先指出可能失效条件

## 失败处理

当回测失败或结果异常时，按顺序排查：

1. 数据是否缺失或时间范围错误
2. 参数是否违反交易规则（tick_size、min_size、min_notional）
3. 手续费/滑点是否不合理
4. 是否出现 rejected/liquidation 且未在结论中披露

若仍无法定位，输出最小复现配置（symbol、区间、参数、命令）后再继续迭代。
