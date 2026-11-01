//! Tier-2 tests: the **live** code path, driven by a scripted exchange.
//!
//! `is_live() == true` makes the engine run exactly what a real live exchange would:
//! `source_level == strategy_level`, no resampling, and gap filling through `get_kline`.
//! A fake exchange therefore exercises live behaviour without any network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use trading_maid::prelude::*;

/// 2024-01-01T00:00:00Z
const T0: u64 = 1_704_067_200_000;
const MIN: u64 = 60_000;

fn kline(time: u64, close: Decimal) -> KLine {
    KLine {
        time,
        open: close,
        high: close,
        low: close,
        close,
        volume: dec!(1),
    }
}

fn metadata() -> Metadata {
    Metadata {
        symbol: "BTCUSDT".to_string(),
        level: Level::Minute1,
        min_size: dec!(0.01),
        min_notional: dec!(0),
        tick_size: dec!(0.1),
        maker_fee: dec!(0.0002),
        taker_fee: dec!(0.0005),
        maintenance: dec!(0.004),
    }
}

/// `stream` is what `next()` yields (it may skip bars); `store` is what `get_kline()` can
/// backfill from. Keeping them separate is what lets a test inject a network gap.
struct FakeLive {
    stream: HashMap<String, Vec<KLine>>,
    store: HashMap<String, Vec<KLine>>,
    index: Mutex<HashMap<String, usize>>,
}

impl FakeLive {
    fn new(stream: Vec<(&str, Vec<KLine>)>, store: Vec<(&str, Vec<KLine>)>) -> Self {
        Self {
            stream: stream
                .into_iter()
                .map(|(symbol, kline_list)| (symbol.to_string(), kline_list))
                .collect(),
            store: store
                .into_iter()
                .map(|(symbol, kline_list)| (symbol.to_string(), kline_list))
                .collect(),
            index: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Exchange for FakeLive {
    fn is_live(&self) -> bool {
        true
    }

    async fn next(&self, symbol: &str, _level: Level) -> anyhow::Result<Option<KLine>> {
        let position = {
            let mut index = self.index.lock().unwrap();
            let counter = index.entry(symbol.to_string()).or_insert(0);
            let position = *counter;
            *counter += 1;
            position
        };

        Ok(self
            .stream
            .get(symbol)
            .and_then(|kline_list| kline_list.get(position))
            .copied())
    }

    async fn get_kline(
        &self,
        symbol: &str,
        _level: Level,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Vec<KLine>> {
        Ok(self
            .store
            .get(symbol)
            .map(|kline_list| {
                kline_list
                    .iter()
                    .filter(|kline| kline.time >= start && kline.time < end)
                    .copied()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn place_order(&self, _order: Order) -> anyhow::Result<String> {
        Ok("1".to_string())
    }

    async fn cancel_order(&self, _symbol: &str, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_all_order(&self, _symbol: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_order(&self, _id: &str) -> anyhow::Result<Option<OrderMessage>> {
        Ok(None)
    }

    async fn get_history_order_list(&self, _symbol: &str) -> anyhow::Result<Vec<OrderMessage>> {
        Ok(Vec::new())
    }

    async fn get_pending_order_list(&self, _symbol: &str) -> anyhow::Result<Vec<OrderMessage>> {
        Ok(Vec::new())
    }

    async fn get_position(&self, _symbol: &str) -> anyhow::Result<Option<Position>> {
        Ok(None)
    }

    async fn close_all_position(&self, _symbol: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_history_position_list(
        &self,
        _symbol: &str,
    ) -> anyhow::Result<Vec<HistoryPosition>> {
        Ok(Vec::new())
    }

    async fn append_position_margin(&self, _symbol: &str, _margin: Decimal) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_equity(&self) -> anyhow::Result<Decimal> {
        Ok(dec!(10000))
    }

    async fn get_cash(&self) -> anyhow::Result<Decimal> {
        Ok(dec!(10000))
    }

    async fn get_leverage(&self, _symbol: &str) -> anyhow::Result<u32> {
        Ok(1)
    }

    async fn set_leverage(&self, _symbol: &str, _leverage: u32) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_metadata(&self, _symbol: &str) -> anyhow::Result<Metadata> {
        Ok(metadata())
    }
}

#[derive(Default, Clone)]
struct Recorder {
    missed: Rc<RefCell<Vec<usize>>>,
    close: Rc<RefCell<Vec<Decimal>>>,
    requested_5m: Rc<RefCell<Vec<Option<Decimal>>>>,
}

impl Recorder {
    fn strategy(&self) -> RecordingStrategy {
        RecordingStrategy {
            missed: self.missed.clone(),
            close: self.close.clone(),
            requested_5m: self.requested_5m.clone(),
        }
    }
}

struct RecordingStrategy {
    missed: Rc<RefCell<Vec<usize>>>,
    close: Rc<RefCell<Vec<Decimal>>>,
    requested_5m: Rc<RefCell<Vec<Option<Decimal>>>>,
}

#[async_trait(?Send)]
impl Strategy for RecordingStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.missed.borrow_mut().push(cx.missed);
        self.close.borrow_mut().push(cx.close[0]);
        self.requested_5m
            .borrow_mut()
            .push(cx.request("BTCUSDT", Level::Minute5).map(|c| c.close[0]));

        Ok(())
    }
}

struct RecordingHook {
    missed: Rc<RefCell<Vec<usize>>>,
}

#[async_trait(?Send)]
impl HookFn for RecordingHook {
    async fn next(
        &mut self,
        _kline: KLine,
        missed_kline_list: &[KLine],
        _exchange: ExchangeWrapper,
    ) -> anyhow::Result<()> {
        self.missed.borrow_mut().push(missed_kline_list.len());

        Ok(())
    }
}

/// 10:00 和 10:03 到达，中间两根靠 `get_kline` 补回来。
fn gapped_exchange() -> FakeLive {
    let store: Vec<KLine> = (0..4u64)
        .map(|i| kline(T0 + i * MIN, Decimal::from(100 + i)))
        .collect();

    let stream = vec![store[0], store[3]];

    FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)])
}

#[tokio::test]
async fn gap_is_reported_as_missed() {
    let recorder = Recorder::default();
    let mut engine = Engine::new(gapped_exchange(), recorder.strategy());

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    // 第一根没有缺口；第二根一次跳了 2 根
    assert_eq!(recorder.missed.borrow().as_slice(), &[0, 2]);

    // 补回来的 bar 就在序列里，所以策略看到的收盘价是连续的
    assert_eq!(recorder.close.borrow().as_slice(), &[dec!(100), dec!(103)]);
}

#[tokio::test]
async fn hook_receives_the_backfilled_kline_list() {
    let recorder = Recorder::default();
    let hook_missed = Rc::new(RefCell::new(Vec::new()));

    let mut engine = Engine::new(gapped_exchange(), recorder.strategy());

    engine.hook(RecordingHook {
        missed: hook_missed.clone(),
    });

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    assert_eq!(hook_missed.borrow().as_slice(), &[0, 2]);
}

#[tokio::test]
async fn unfillable_gap_aborts_the_run() {
    // 10:00 和 10:03 到达，但 store 里也没有中间那两根
    let store = vec![kline(T0, dec!(100)), kline(T0 + 3 * MIN, dec!(103))];
    let stream = store.clone();

    let exchange = FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)]);

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    let error = engine.run("BTCUSDT", Level::Minute1).await.unwrap_err();

    assert!(
        format!("{error}").contains("time discontinuity"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn request_sees_the_forming_bar_of_a_coarser_level() {
    let kline_list: Vec<KLine> = (0..5u64)
        .map(|i| kline(T0 + i * MIN, Decimal::from(100 + i)))
        .collect();

    let exchange = FakeLive::new(
        vec![
            ("BTCUSDT", kline_list.clone()),
            ("ETHUSDT", kline_list.clone()),
        ],
        vec![
            ("BTCUSDT", kline_list.clone()),
            ("ETHUSDT", kline_list.clone()),
        ],
    );

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    engine
        .run(
            vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()],
            Level::Minute1,
        )
        .await
        .unwrap();

    let requested = recorder.requested_5m.borrow();

    assert_eq!(requested.len(), 5);

    // 5m 周期的半成品 close 跟着最新一根 1m 走
    assert_eq!(
        requested.as_slice(),
        &[
            Some(dec!(100)),
            Some(dec!(101)),
            Some(dec!(102)),
            Some(dec!(103)),
            Some(dec!(104)),
        ]
    );
}
