use trading_maid::prelude::*;

/// Holds outstanding order IDs per symbol so we can cancel them individually
/// when the position closes, instead of calling `cancel_all_order`.
struct MyStrategy {
    btc: Option<SymbolOrders>,
    eth: Option<SymbolOrders>,
}

struct SymbolOrders {
    master_id: String,
    tp_id: Option<String>,
    sl_id: Option<String>,
}

impl SymbolOrders {
    async fn cancel(self, cx: &Context<'_>, symbol: &str) {
        _ = cx.cancel_order(symbol, &self.master_id).await;
        if let Some(id) = self.tp_id {
            _ = cx.cancel_order(symbol, &id).await;
        }
        if let Some(id) = self.sl_id {
            _ = cx.cancel_order(symbol, &id).await;
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Strategy for MyStrategy {
    async fn next(&mut self, cx: &Context) -> anyhow::Result<()> {
        // ---- BTCUSDT ----
        if cx.get_position("BTCUSDT").await?.is_none() {
            // Position gone → cancel any leftover orders by id.
            if let Some(orders) = self.btc.take() {
                orders.cancel(cx, "BTCUSDT").await;
            }

            // Check entry condition on BTCUSDT's own OHLCV.
            if let Some((tp, sl)) = entry_signal(cx) {
                println!("BTCUSDT: {}", t2s(cx.time));

                let (master, tp_r, sl_r) = cx.sell_tp_sl("BTCUSDT", tp, sl, dec!(0.01)).await?;

                self.btc = Some(SymbolOrders {
                    master_id: master,
                    tp_id: tp_r.ok(),
                    sl_id: sl_r.ok(),
                });
            }
        }

        // ---- ETHUSDT ----
        if cx.get_position("ETHUSDT").await?.is_none() {
            if let Some(orders) = self.eth.take() {
                orders.cancel(cx, "ETHUSDT").await;
            }

            // Read ETHUSDT's own OHLCV via multi-symbol request.
            if let Some(eth_cx) = cx.request("ETHUSDT", Level::Hour1) {
                if let Some((tp, sl)) = entry_signal(&eth_cx) {
                    println!("ETHUSDT: {}", t2s(cx.time));

                    let (master, tp_r, sl_r) = cx.sell_tp_sl("ETHUSDT", tp, sl, dec!(0.01)).await?;

                    self.eth = Some(SymbolOrders {
                        master_id: master,
                        tp_id: tp_r.ok(),
                        sl_id: sl_r.ok(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Returns `(take_profit_price, stop_loss_price)` when the short-entry
/// condition is met on the given context's bar.
fn entry_signal(cx: &Context<'_>) -> Option<(Decimal, Decimal)> {
    let body_size = (cx.open - cx.close).abs();
    let upper_shadow_size = (cx.high - cx.open).abs();
    let threshold = cx.open * dec!(0.003); // 0.3% of price, works for any symbol

    let condition =
        cx.open > cx.close && upper_shadow_size >= body_size * 2 && body_size >= threshold;

    if condition {
        Some((cx.open - upper_shadow_size, cx.open + upper_shadow_size))
    } else {
        None
    }
}

// cargo test -r --test multi -- --ignored
#[ignore]
#[tokio::test]
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

    let path2 = get_or_download("ETHUSDT/1m", 12).await.unwrap();

    let data_source_1m_eth = DataSource::from_file_metadata(
        path2,
        Metadata {
            symbol: "ETHUSDT".to_string(),
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

    let exchange = LocalExchange::new([data_source_1m.clone(), data_source_1m_eth.clone()])
        .cash(10000)
        .leverage(10)
        .slippage(0);

    let mut engine = Engine::new(
        exchange.clone(),
        MyStrategy {
            btc: None,
            eth: None,
        },
    );

    if let Err(v) = engine.run(["BTCUSDT", "ETHUSDT"], Level::Hour1).await {
        println!("error: {:#?}", v);
    }

    let mut history_position = exchange.get_history_position_list("BTCUSDT").await.unwrap();
    let mut history_order = exchange.get_history_order_list("BTCUSDT").await.unwrap();

    history_position.extend(exchange.get_history_position_list("ETHUSDT").await.unwrap());
    history_order.extend(exchange.get_history_order_list("ETHUSDT").await.unwrap());

    let summary = summarize(&history_position);

    println!("history summary: {:#?}", summary);

    let data_source_1h = data_source_1m.resample(Level::Hour1).unwrap();
    let data_source_1h_eth = data_source_1m_eth.resample(Level::Hour1).unwrap();

    open_in_server(
        [
            data_source_1h,
            data_source_1m,
            data_source_1m_eth,
            data_source_1h_eth,
        ],
        history_position,
        history_order,
    )
    .await
    .unwrap();
}
