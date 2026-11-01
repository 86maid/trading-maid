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

    async fn get_order(&self, _symbol: &str, _id: &str) -> anyhow::Result<Option<OrderMessage>> {
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
    time0: Rc<RefCell<Vec<u64>>>,
    length: Rc<RefCell<Vec<usize>>>,
    time_all: Rc<RefCell<Vec<Vec<u64>>>>,
}

impl Recorder {
    fn strategy(&self) -> RecordingStrategy {
        RecordingStrategy {
            missed: self.missed.clone(),
            close: self.close.clone(),
            requested_5m: self.requested_5m.clone(),
            time0: self.time0.clone(),
            length: self.length.clone(),
            time_all: self.time_all.clone(),
        }
    }
}

struct RecordingStrategy {
    missed: Rc<RefCell<Vec<usize>>>,
    close: Rc<RefCell<Vec<Decimal>>>,
    requested_5m: Rc<RefCell<Vec<Option<Decimal>>>>,
    time0: Rc<RefCell<Vec<u64>>>,
    length: Rc<RefCell<Vec<usize>>>,
    time_all: Rc<RefCell<Vec<Vec<u64>>>>,
}

#[async_trait(?Send)]
impl Strategy for RecordingStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        self.missed.borrow_mut().push(cx.missed);
        self.close.borrow_mut().push(cx.close[0]);
        self.time0.borrow_mut().push(cx.time[0]);
        self.length.borrow_mut().push(cx.close.len());
        self.time_all
            .borrow_mut()
            .push((0..cx.time.len()).map(|i| cx.time[i]).collect());
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

// ---------------------------------------------------------------------------
// preload
// ---------------------------------------------------------------------------

/// `count` 根连续的 1m K 线，从 `start` 开始，收盘价依次递增。
fn bar_list(start: u64, count: usize) -> Vec<KLine> {
    (0..count)
        .map(|i| kline(start + i as u64 * MIN, Decimal::from(100 + i as i64)))
        .collect()
}

/// `store` 里是完整历史，`stream` 里只吐出最后一根（模拟「启动时收到的第一根」）。
fn preload_exchange(store_len: usize, stream_index: usize) -> FakeLive {
    let store = bar_list(T0, store_len);
    let stream = vec![store[stream_index]];

    FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)])
}

#[tokio::test]
async fn preload_fills_history_before_the_first_live_bar() {
    // store: T0..T0+9，live 只收到 T0+9
    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    engine.preload(3);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    // 3 根预加载 + 1 根 live，且时间上严格连续
    assert_eq!(recorder.time0.borrow().as_slice(), &[T0 + 9 * MIN]);
    assert_eq!(recorder.length.borrow().as_slice(), &[4]);
    assert_eq!(recorder.close.borrow().as_slice(), &[dec!(109)],);
}

#[tokio::test]
async fn preload_series_is_contiguous_up_to_the_first_live_bar() {
    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    engine.preload(3);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    // 策略看到的第一根是 T0+9，往前正好 3 根且时间严格连续：T0+8、T0+7、T0+6
    assert_eq!(
        recorder.time_all.borrow().as_slice(),
        &[vec![T0 + 9 * MIN, T0 + 8 * MIN, T0 + 7 * MIN, T0 + 6 * MIN,]]
    );
}

#[tokio::test]
async fn preload_from_starts_at_the_given_bar() {
    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    // 从 T0+5 开始，一直到第一根 live K 线 T0+9 之前：T0+5..T0+8 共 4 根
    engine.preload_from(T0 + 5 * MIN);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    assert_eq!(recorder.length.borrow().as_slice(), &[5]);
}

#[tokio::test]
async fn preload_from_after_the_first_live_bar_is_a_noop() {
    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    // 起点不早于第一根 live K 线 → 没有可预加载的内容，也不应报错
    engine.preload_from(T0 + 20 * MIN);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    assert_eq!(recorder.length.borrow().as_slice(), &[1]);
}

#[tokio::test]
async fn preload_happens_only_once() {
    let store = bar_list(T0, 11);
    let stream = vec![store[9], store[10]];

    let exchange = FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)]);

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    engine.preload(3);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    // 第一根：3 预加载 + 1；第二根只多 1 根（不会再预加载一次）
    assert_eq!(recorder.length.borrow().as_slice(), &[4, 5]);
}

#[tokio::test]
async fn preload_data_seeds_from_memory_and_fills_the_rest() {
    // 种子只有 T0..T0+5，live 从 T0+8 开始 → T0+6、T0+7 由 get_kline 补
    let seed = DataSource::new(metadata(), bar_list(T0, 6));

    let store = bar_list(T0, 9);
    let stream = vec![store[8]];

    let exchange = FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)]);

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    engine.preload_data(seed);

    engine.run("BTCUSDT", Level::Minute1).await.unwrap();

    // 6 根种子 + 2 根补齐 + 1 根 live
    assert_eq!(recorder.length.borrow().as_slice(), &[9]);
    assert_eq!(recorder.time0.borrow().as_slice(), &[T0 + 8 * MIN]);
}

#[tokio::test]
async fn preload_data_resamples_a_finer_seed() {
    // 种子是 1m，策略是 5m（live 下 level 就是策略级别），种子需要被重采样上去
    let mut seed_metadata = metadata();
    seed_metadata.level = Level::Minute1;

    // 1m 种子：T0 .. T0+7，重采样成 5m 后是 T0 与 T0+5 两根
    let seed = DataSource::new(seed_metadata, bar_list(T0, 8));

    // 5m 的完整历史：T0、T0+5、T0+10、T0+15、T0+20
    let store: Vec<KLine> = (0..5).map(|i| kline(T0 + i * 5 * MIN, dec!(200))).collect();

    // 第一根 live K 线在 T0+20
    let stream = vec![store[4]];

    let exchange = FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)]);

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    engine.preload_data(seed);

    engine.run("BTCUSDT", Level::Minute5).await.unwrap();

    // 种子重采样出 T0、T0+5；补齐 T0+10、T0+15；再加 live 的 T0+20
    assert_eq!(
        recorder.time_all.borrow().as_slice(),
        &[vec![
            T0 + 20 * MIN,
            T0 + 15 * MIN,
            T0 + 10 * MIN,
            T0 + 5 * MIN,
            T0,
        ]]
    );
}

#[tokio::test]
async fn preload_data_rejects_a_coarser_seed() {
    // 种子是 5m，策略是 1m：无法向下采样，必须报错而不是拿错级别的数据预热
    let mut seed_metadata = metadata();
    seed_metadata.level = Level::Minute5;

    let seed = DataSource::new(
        seed_metadata,
        (0..4).map(|i| kline(T0 + i * 5 * MIN, dec!(100))).collect(),
    );

    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    engine.preload_data(seed);

    let error = engine.run("BTCUSDT", Level::Minute1).await.unwrap_err();

    assert!(
        format!("{error}").contains("cannot resample"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn preload_data_rejects_a_different_symbol() {
    let mut wrong = metadata();
    wrong.symbol = "ETHUSDT".to_string();

    let seed = DataSource::new(wrong, bar_list(T0, 3));

    let recorder = Recorder::default();
    let mut engine = Engine::new(preload_exchange(10, 9), recorder.strategy());

    engine.preload_data(seed);

    let error = engine.run("BTCUSDT", Level::Minute1).await.unwrap_err();

    assert!(
        format!("{error}").contains("symbol mismatch"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn preload_rejects_a_hole_right_before_the_first_live_bar() {
    // store 只到 T0+8，live 却在 T0+10 → T0+9 补不出来，必须报错
    let store = bar_list(T0, 9);
    let stream = vec![kline(T0 + 10 * MIN, dec!(999))];

    let exchange = FakeLive::new(vec![("BTCUSDT", stream)], vec![("BTCUSDT", store)]);

    let recorder = Recorder::default();
    let mut engine = Engine::new(exchange, recorder.strategy());

    engine.preload(3);

    let error = engine.run("BTCUSDT", Level::Minute1).await.unwrap_err();

    assert!(
        format!("{error}").contains("is not adjacent"),
        "unexpected error: {error}"
    );
}
