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




const INITIAL_CASH: Decimal = dec!(1000000);
const SYMBOL: &str = "BTCUSDT";
const LEVEL: Level = Level::Hour4;


fn print_report(
    month_count: u32,
    symbol: &str,
    level: Level,
    result: &BacktestResult,
    drawdown: Decimal,
) {
    let summary = result.summarize();

    println!("== {} 个月 · {} · {} ==", month_count, symbol, level);
    println!("summary: {:#?}", summary);
    println!(
        "total_trades={}  win_rate={}%  profit_loss_ratio={}  avg_profit={}  total_profit={}  max_drawdown={}",
        summary.total_trades,
        summary.win_rate.round_dp(2),
        summary.profit_loss_ratio.round_dp(2),
        summary.avg_profit.round_dp(2),
        summary.total_profit.round_dp(2),
        drawdown.round_dp(2),
    );
    println!();
}

// backtest() 内部不给挂 hook，所以这里手工搭一遍同样的流程。
#[tokio::main]
async fn main() {
    let mut last = None;

    // 同一份代码跑 8 个窗口（参数未按窗口调整），用于观察稳健性。
    for month_count in [3u32, 6, 12, 24, 36, 48, 60, 72] {
        let data_source = DataSource::from_file_metadata(
            get_or_download(format!("{}/1m", SYMBOL), month_count)
                .await
                .unwrap(),
            Metadata {
                symbol: SYMBOL.to_string(),
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

        let exchange = LocalExchange::new(data_source.clone())
            .unwrap()
            .cash(INITIAL_CASH);

        let drawdown = DrawdownHook::new(INITIAL_CASH);

        let mut engine = Engine::new(exchange.clone(), 
Momentum::new()
);
        engine.hook(drawdown.clone());
        engine.run(SYMBOL, LEVEL).await.unwrap();

        let history_position = exchange.get_history_position_list(SYMBOL).await.unwrap();
        let history_order = exchange.get_history_order_list(SYMBOL).await.unwrap();

        let result = BacktestResult {
            data_source,
            history_position,
            history_order,
        };

        print_report(month_count, SYMBOL, LEVEL, &result, drawdown.get());

        last = Some(result);
    }
    // 数据重采样到所有级别后可视化，只对最后一个窗口开一次
    if let Some(result) = last {
        result.resample_all_open_in_server().await.unwrap();
    }

}
