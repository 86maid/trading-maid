use std::ops::RangeBounds;
use std::sync::Arc;

use anyhow::Context;
use rust_decimal::Decimal;

use crate::data::*;
use crate::order::*;

/// Exchange is the core abstraction over both backtesting and live trading environments.
///
/// It defines a unified interface for:
/// - advancing time and retrieving k-lines ([`next`](Exchange::next))
/// - placing and managing orders ([`place_order`](Exchange::place_order), [`cancel_order`](Exchange::cancel_order))
/// - querying positions, orders, equity, and metadata
///
/// The trait provides **convenience methods** (e.g. [`buy`](Exchange::buy), [`sell_limit`](Exchange::sell_limit),
/// [`sell_tp_sl`](Exchange::sell_tp_sl)) with sensible defaults that delegate to [`place_order`](Exchange::place_order).
/// Implementors only need to provide the core methods; the convenience methods can be overridden
/// for exchange-specific optimization.
///
/// # Naming convention
///
/// `{side}_{order_type}_{reduce_only}` where:
/// - `side`: `buy` or `sell`
/// - `order_type`: `market` (omitted), `limit`, `trigger_limit`, `trigger_market`
/// - `reduce_only`: appended to mean the order won't open or flip a position
///
/// # Market vs Limit vs Trigger
///
/// | Type | `trigger_price` | `price` | Fills |
/// |------|:---------------:|:-------:|-------|
/// | Market | ZERO | ZERO | at `Open` on next k-line |
/// | Limit | ZERO | > 0 | at `price` or better on next k-line |
/// | Trigger Market | > 0 | ZERO | market fill immediately when trigger hit |
/// | Trigger Limit | > 0 | > 0 | limit fill immediately when trigger hit |
#[async_trait::async_trait]
pub trait Exchange
where
    Self: Send + Sync + 'static,
{
    /// Gets the latest K-line data for the current trading symbol.
    ///
    /// # Behavior
    /// - This method always returns the latest K-line at the current time.
    /// - If the current time has not yet reached the generation time of the next K-line
    ///   (i.e., a complete `level` cycle has not elapsed),
    ///   the call will **block** until a new K-line is generated.
    /// - The returned K-line data should be temporally continuous with the K-line returned
    ///   from the previous call.
    ///
    /// # Parameters
    /// - `symbol`: The trading symbol, e.g., `"BTCUSDT"`.
    /// - `level`: The K-line period level, e.g., `Level::Hour1` for 1-hour K-lines.
    ///
    /// # Returns
    /// - `Ok(Some(KLine))`: Successfully retrieved the latest K-line.
    /// - `Ok(None)`: No data available (e.g., the trading pair does not exist or the data source is unavailable).
    /// - `Err`: An error occurred.
    ///
    /// # Note
    /// If the returned K-line timestamp is not continuous with the previous K-line timestamp
    /// (i.e., there is a time gap),
    /// the backtesting engine will automatically call [`get_kline`] to fill in the missing data segments.
    async fn next(&self, symbol: &str, level: Level) -> anyhow::Result<Option<KLine>>;

    /// Batch retrieves historical K-line data within a specified time range.
    ///
    /// # Behavior
    /// - Returns all K-line data within the time range `[start, end)`, i.e., inclusive of the K-line
    ///   at `start`, exclusive of the K-line at `end`.
    /// - The returned list of K-lines should be sorted in **ascending** order by time.
    /// - This method is primarily used by the backtesting engine to automatically fill in missing data
    ///   when a discontinuity in K-line timestamps is detected.
    ///
    /// # Parameters
    /// - `symbol`: The trading symbol, e.g., `"BTCUSDT"`.
    /// - `level`: The K-line period level, e.g., `Level::Hour1` for 1-hour K-lines.
    /// - `start`: Start timestamp (inclusive), in milliseconds or seconds (must be consistent with the data source).
    /// - `end`: End timestamp (exclusive), in milliseconds or seconds (must be consistent with the data source).
    ///
    /// # Returns
    /// - `Ok(Vec<KLine>)`: A list of K-lines within the time range, or an empty vector if no data is available.
    /// - `Err`: An error occurred.
    async fn get_kline(
        &self,
        symbol: &str,
        level: Level,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Vec<KLine>>;

    /// Submit a raw [`Order`] to the exchange.
    ///
    /// Returns the order ID on success. The order may be rejected immediately
    /// (e.g. price not aligned to tick_size, insufficient margin).
    async fn place_order(&self, order: Order) -> anyhow::Result<String>;

    /// Cancel a single pending order by its ID.
    async fn cancel_order(&self, symbol: &str, id: &str) -> anyhow::Result<()>;

    /// Cancel all pending orders for the given symbol.
    async fn cancel_all_order(&self, symbol: &str) -> anyhow::Result<()>;

    /// Query an order by ID. Returns `None` if not found.
    async fn get_order(&self, id: &str) -> anyhow::Result<Option<OrderMessage>>;

    /// Return all historical orders (filled, canceled, rejected) for the symbol.
    async fn get_history_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>>;

    /// Return all currently pending (unfilled) orders for the symbol.
    async fn get_pending_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>>;

    /// Return the current open position for the symbol, or `None` if no position.
    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>>;

    /// Close the entire position for the symbol at market price.
    async fn close_all_position(&self, symbol: &str) -> anyhow::Result<()>;

    /// Return the history of all closed positions for the symbol.
    async fn get_history_position_list(&self, symbol: &str)
    -> anyhow::Result<Vec<HistoryPosition>>;

    /// Add margin to an existing position. Dynamic margin management only.
    async fn append_position_margin(&self, symbol: &str, margin: Decimal) -> anyhow::Result<()>;

    /// Return current equity (cash + unrealized PnL).
    async fn get_equity(&self) -> anyhow::Result<Decimal>;

    /// Return current available cash balance.
    async fn get_cash(&self) -> anyhow::Result<Decimal>;

    /// Return the current leverage for the symbol.
    async fn get_leverage(&self, symbol: &str) -> anyhow::Result<u32>;

    /// Set the leverage for the symbol. Only takes effect on the next position open.
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()>;

    /// Return the trading metadata (tick_size, min_size, fees, etc.) for the symbol.
    async fn get_metadata(&self, symbol: &str) -> anyhow::Result<Metadata>;

    /// Place a market buy order. Fills at `Open` on the next k-line.
    ///
    /// This opens or increases a long position. Not reduce-only.
    async fn buy(&self, symbol: &str, quantity: Decimal) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: Decimal::ZERO,
            price: Decimal::ZERO,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a market sell order. Fills at `Open` on the next k-line.
    ///
    /// This opens or increases a short position. Not reduce-only.
    async fn sell(&self, symbol: &str, quantity: Decimal) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: Decimal::ZERO,
            price: Decimal::ZERO,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a limit buy order. Fills at `price` or better on the next k-line.
    ///
    /// - If `price >= market`: fills at worst price `High`.
    /// - If `price < market`: fills at `price`.
    async fn buy_limit(
        &self,
        symbol: &str,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: Decimal::ZERO,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a limit sell order. Fills at `price` or better on the next k-line.
    ///
    /// - If `price <= market`: fills at worst price `Low`.
    /// - If `price > market`: fills at `price`.
    async fn sell_limit(
        &self,
        symbol: &str,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: Decimal::ZERO,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a trigger-limit buy order.
    ///
    /// When the price reaches `trigger_price`, a limit buy at `price` is placed
    /// and matched immediately on the current k-line.
    async fn buy_trigger_limit(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a trigger-limit sell order.
    ///
    /// When the price reaches `trigger_price`, a limit sell at `price` is placed
    /// and matched immediately on the current k-line.
    async fn sell_trigger_limit(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a trigger-market buy order.
    ///
    /// When the price reaches `trigger_price`, a market buy is executed immediately
    /// on the current k-line at `Open`.
    async fn buy_trigger_market(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price: Decimal::ZERO,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Place a trigger-market sell order.
    ///
    /// When the price reaches `trigger_price`, a market sell is executed immediately
    /// on the current k-line at `Open`.
    async fn sell_trigger_market(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price: Decimal::ZERO,
            quantity,
            reduce_only: false,
        })
        .await
    }

    /// Market buy with `reduce_only`. Only reduces an existing short position;
    /// won't open a new long or flip the position.
    async fn buy_reduce_only(&self, symbol: &str, quantity: Decimal) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: Decimal::ZERO,
            price: Decimal::ZERO,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Market sell with `reduce_only`. Only reduces an existing long position;
    /// won't open a new short or flip the position.
    async fn sell_reduce_only(&self, symbol: &str, quantity: Decimal) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: Decimal::ZERO,
            price: Decimal::ZERO,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Limit buy with `reduce_only`.
    async fn buy_limit_reduce_only(
        &self,
        symbol: &str,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: Decimal::ZERO,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Limit sell with `reduce_only`.
    async fn sell_limit_reduce_only(
        &self,
        symbol: &str,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: Decimal::ZERO,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Trigger-limit buy with `reduce_only`.
    async fn buy_trigger_limit_reduce_only(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Trigger-limit sell with `reduce_only`.
    async fn sell_trigger_limit_reduce_only(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Trigger-market buy with `reduce_only`. Commonly used for stop loss on a short position.
    async fn buy_trigger_market_reduce_only(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price: Decimal::ZERO,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Trigger-market sell with `reduce_only`. Commonly used for stop loss on a long position.
    async fn sell_trigger_market_reduce_only(
        &self,
        symbol: &str,
        trigger_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price: Decimal::ZERO,
            quantity,
            reduce_only: true,
        })
        .await
    }

    /// Open a long position with take-profit and stop-loss orders (syntactic sugar).
    ///
    /// Places three orders concurrently:
    /// 1. A **market buy** to open the position.
    /// 2. A **limit sell** at `tp_price` (take-profit, reduce-only).
    /// 3. A **trigger-market sell** at `sl_price` (stop-loss, reduce-only).
    ///
    /// Returns `(master_order_id, take_profit_result, stop_loss_result)`.
    /// Both TP and SL are `reduce_only` and won't flip the position.
    async fn buy_tp_sl(
        &self,
        symbol: &str,
        tp_price: Decimal,
        sl_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<(String, anyhow::Result<String>, anyhow::Result<String>)> {
        let master_order_id = self
            .place_order(Order {
                symbol: symbol.to_string(),
                side: Side::Buy,
                trigger_price: Decimal::ZERO,
                price: Decimal::ZERO,
                quantity,
                reduce_only: false,
            })
            .await?;

        let (take_profit_result, stop_loss_result) = tokio::join!(
            async move {
                let result: anyhow::Result<String> = async {
                    let order_id = self
                        .place_order(Order {
                            symbol: symbol.to_string(),
                            side: Side::Sell,
                            trigger_price: Decimal::ZERO,
                            price: tp_price,
                            quantity,
                            reduce_only: true,
                        })
                        .await?;

                    self.get_order(&order_id)
                        .await?
                        .context("take profit order not found")?
                        .status
                        .ok()?;

                    Ok(order_id)
                }
                .await;

                result
            },
            async move {
                let result: anyhow::Result<String> = async {
                    let order_id = self
                        .place_order(Order {
                            symbol: symbol.to_string(),
                            side: Side::Sell,
                            trigger_price: sl_price,
                            price: Decimal::ZERO,
                            quantity,
                            reduce_only: true,
                        })
                        .await?;

                    self.get_order(&order_id)
                        .await?
                        .context("stop loss order not found")?
                        .status
                        .ok()?;

                    Ok(order_id)
                }
                .await;

                result
            }
        );

        Ok((master_order_id, take_profit_result, stop_loss_result))
    }

    /// Open a short position with take-profit and stop-loss orders (syntactic sugar).
    ///
    /// Places three orders concurrently:
    /// 1. A **market sell** to open the position.
    /// 2. A **limit buy** at `tp_price` (take-profit, reduce-only).
    /// 3. A **trigger-market buy** at `sl_price` (stop-loss, reduce-only).
    ///
    /// Returns `(master_order_id, take_profit_result, stop_loss_result)`.
    /// Both TP and SL are `reduce_only` and won't flip the position.
    async fn sell_tp_sl(
        &self,
        symbol: &str,
        tp_price: Decimal,
        sl_price: Decimal,
        quantity: Decimal,
    ) -> anyhow::Result<(String, anyhow::Result<String>, anyhow::Result<String>)> {
        let master_order_id = self
            .place_order(Order {
                symbol: symbol.to_string(),
                side: Side::Sell,
                trigger_price: Decimal::ZERO,
                price: Decimal::ZERO,
                quantity,
                reduce_only: false,
            })
            .await?;

        let (take_profit_result, stop_loss_result) = tokio::join!(
            async move {
                let result: anyhow::Result<String> = async {
                    let order_id = self
                        .place_order(Order {
                            symbol: symbol.to_string(),
                            side: Side::Buy,
                            trigger_price: Decimal::ZERO,
                            price: tp_price,
                            quantity,
                            reduce_only: true,
                        })
                        .await?;

                    self.get_order(&order_id)
                        .await?
                        .context("take profit order not found")?
                        .status
                        .ok()?;

                    Ok(order_id)
                }
                .await;

                result
            },
            async move {
                let result: anyhow::Result<String> = async {
                    let order_id = self
                        .place_order(Order {
                            symbol: symbol.to_string(),
                            side: Side::Buy,
                            trigger_price: sl_price,
                            price: Decimal::ZERO,
                            quantity,
                            reduce_only: true,
                        })
                        .await?;

                    self.get_order(&order_id)
                        .await?
                        .context("stop loss order not found")?
                        .status
                        .ok()?;

                    Ok(order_id)
                }
                .await;

                result
            }
        );

        Ok((master_order_id, take_profit_result, stop_loss_result))
    }
}

/// A user-friendly wrapper around [`Exchange`] that accepts `impl AsRef<str>` for
/// symbol/id parameters and `impl TryInto<Decimal>` for numeric parameters.
///
/// This is the type exposed via [`Context`](crate::context::Context) (through `Deref`),
/// so strategy code calls methods directly on `cx`.
///
/// # Parameter flexibility
///
/// `symbol` and `id` accept both `&str` and `String`.
/// `price`, `quantity`, `trigger_price` accept `&str`, `f64`, `Decimal`, etc.
/// — anything that implements [`TryInto<Decimal>`].
///
/// **Use `&str` for high-precision values** (e.g. `"0.01"`) to avoid `f64` precision loss.
#[derive(Clone)]
pub struct ExchangeWrapper(Arc<dyn Exchange + 'static>);

impl ExchangeWrapper {
    /// Wrap an [`Exchange`] implementation. Called internally by [`Engine`](crate::engine::Engine).
    pub fn new(exchange: Arc<dyn Exchange + 'static>) -> Self {
        Self(exchange)
    }

    #[allow(dead_code)]
    pub(crate) async fn next(
        &self,
        symbol: impl AsRef<str>,
        level: Level,
    ) -> anyhow::Result<Option<KLine>> {
        self.0.next(symbol.as_ref(), level).await
    }

    #[allow(dead_code)]
    pub(crate) async fn get_kline(
        &self,
        symbol: impl AsRef<str>,
        level: Level,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Vec<KLine>> {
        self.0.get_kline(symbol.as_ref(), level, start, end).await
    }

    /// Submit a raw [`Order`]. Prefer the convenience methods ([`buy`](ExchangeWrapper::buy), etc.)
    /// unless you need full control over the order fields.
    pub async fn place_order(&self, order: Order) -> anyhow::Result<String> {
        self.0.place_order(order).await
    }

    /// Cancel a single pending order by ID.
    pub async fn cancel_order(
        &self,
        symbol: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        self.0.cancel_order(symbol.as_ref(), id.as_ref()).await
    }

    /// Cancel all pending orders for the symbol.
    pub async fn cancel_all_order(&self, symbol: impl AsRef<str>) -> anyhow::Result<()> {
        self.0.cancel_all_order(symbol.as_ref()).await
    }

    /// Query an order by ID. Returns `None` if not found.
    pub async fn get_order(&self, id: impl AsRef<str>) -> anyhow::Result<Option<OrderMessage>> {
        self.0.get_order(id.as_ref()).await
    }

    /// Return all historical orders (filled, canceled, rejected) for the symbol.
    pub async fn get_history_order_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<OrderMessage>> {
        self.0.get_history_order_list(symbol.as_ref()).await
    }

    /// Return all currently pending (unfilled) orders for the symbol.
    pub async fn get_pending_order_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<OrderMessage>> {
        self.0.get_pending_order_list(symbol.as_ref()).await
    }

    /// Return the current open position, or `None` if no position.
    pub async fn get_position(&self, symbol: impl AsRef<str>) -> anyhow::Result<Option<Position>> {
        self.0.get_position(symbol.as_ref()).await
    }

    /// Close the entire position for the symbol at market price.
    pub async fn close_all_position(&self, symbol: impl AsRef<str>) -> anyhow::Result<()> {
        self.0.close_all_position(symbol.as_ref()).await
    }

    /// Return the history of all closed positions for the symbol.
    pub async fn get_history_position_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<HistoryPosition>> {
        self.0.get_history_position_list(symbol.as_ref()).await
    }

    /// Add margin to an existing position.
    pub async fn append_position_margin(
        &self,
        symbol: impl AsRef<str>,
        margin: impl TryInto<Decimal>,
    ) -> anyhow::Result<()> {
        self.0
            .append_position_margin(
                symbol.as_ref(),
                margin
                    .try_into()
                    .ok()
                    .context("invalid margin for append_position_margin")?,
            )
            .await
    }

    /// Return current equity (cash + unrealized PnL).
    pub async fn get_equity(&self) -> anyhow::Result<Decimal> {
        self.0.get_equity().await
    }

    /// Return current available cash balance.
    pub async fn get_cash(&self) -> anyhow::Result<Decimal> {
        self.0.get_cash().await
    }

    /// Return the current leverage for the symbol.
    pub async fn get_leverage(&self, symbol: impl AsRef<str>) -> anyhow::Result<u32> {
        self.0.get_leverage(symbol.as_ref()).await
    }

    /// Set the leverage for the symbol. Only takes effect on the next position open.
    pub async fn set_leverage(&self, symbol: impl AsRef<str>, leverage: u32) -> anyhow::Result<()> {
        self.0.set_leverage(symbol.as_ref(), leverage).await
    }

    /// Return the trading metadata (tick_size, min_size, fees, etc.) for the symbol.
    pub async fn get_metadata(&self, symbol: impl AsRef<str>) -> anyhow::Result<Metadata> {
        self.0.get_metadata(symbol.as_ref()).await
    }

    /// Place a market buy order. Fills at `Open` on the next k-line.
    ///
    /// This opens or increases a long position. Not reduce-only.
    pub async fn buy(
        &self,
        symbol: impl AsRef<str>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy(
                symbol.as_ref(),
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy")?,
            )
            .await
    }

    /// Place a market sell order. Fills at `Open` on the next k-line.
    ///
    /// This opens or increases a short position. Not reduce-only.
    pub async fn sell(
        &self,
        symbol: impl AsRef<str>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell(
                symbol.as_ref(),
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell")?,
            )
            .await
    }

    /// Place a limit buy order. Fills at `price` or better on the next k-line.
    ///
    /// - If `price >= market`: fills at worst price `High`.
    /// - If `price < market`: fills at `price`.
    pub async fn buy_limit(
        &self,
        symbol: impl AsRef<str>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_limit(
                symbol.as_ref(),
                price
                    .try_into()
                    .ok()
                    .context("invalid price for buy_limit")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_limit")?,
            )
            .await
    }

    /// Place a limit sell order. Fills at `price` or better on the next k-line.
    ///
    /// - If `price <= market`: fills at worst price `Low`.
    /// - If `price > market`: fills at `price`.
    pub async fn sell_limit(
        &self,
        symbol: impl AsRef<str>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_limit(
                symbol.as_ref(),
                price
                    .try_into()
                    .ok()
                    .context("invalid price for sell_limit")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_limit")?,
            )
            .await
    }

    /// Place a trigger-limit buy order.
    ///
    /// When the price reaches `trigger_price`, a limit buy at `price` is placed
    /// and matched immediately on the current k-line.
    pub async fn buy_trigger_limit(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_trigger_limit(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for buy_trigger_limit")?,
                price
                    .try_into()
                    .ok()
                    .context("invalid price for buy_trigger_limit")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_trigger_limit")?,
            )
            .await
    }

    /// Place a trigger-limit sell order.
    ///
    /// When the price reaches `trigger_price`, a limit sell at `price` is placed
    /// and matched immediately on the current k-line.
    pub async fn sell_trigger_limit(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_trigger_limit(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for sell_trigger_limit")?,
                price
                    .try_into()
                    .ok()
                    .context("invalid price for sell_trigger_limit")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_trigger_limit")?,
            )
            .await
    }

    /// Place a trigger-market buy order.
    ///
    /// When the price reaches `trigger_price`, a market buy is executed immediately
    /// on the current k-line at `Open`.
    pub async fn buy_trigger_market(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_trigger_market(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for buy_trigger_market")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_trigger_market")?,
            )
            .await
    }

    /// Place a trigger-market sell order.
    ///
    /// When the price reaches `trigger_price`, a market sell is executed immediately
    /// on the current k-line at `Open`.
    pub async fn sell_trigger_market(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_trigger_market(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for sell_trigger_market")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_trigger_market")?,
            )
            .await
    }

    /// Market buy with `reduce_only`. Only reduces an existing short position;
    /// won't open a new long or flip the position.
    pub async fn buy_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_reduce_only(
                symbol.as_ref(),
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_reduce_only")?,
            )
            .await
    }

    /// Market sell with `reduce_only`. Only reduces an existing long position;
    /// won't open a new short or flip the position.
    pub async fn sell_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_reduce_only(
                symbol.as_ref(),
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_reduce_only")?,
            )
            .await
    }

    /// Limit buy with `reduce_only`.
    pub async fn buy_limit_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_limit_reduce_only(
                symbol.as_ref(),
                price
                    .try_into()
                    .ok()
                    .context("invalid price for buy_limit_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_limit_reduce_only")?,
            )
            .await
    }

    /// Limit sell with `reduce_only`.
    pub async fn sell_limit_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_limit_reduce_only(
                symbol.as_ref(),
                price
                    .try_into()
                    .ok()
                    .context("invalid price for sell_limit_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_limit_reduce_only")?,
            )
            .await
    }

    /// Trigger-limit buy with `reduce_only`.
    pub async fn buy_trigger_limit_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_trigger_limit_reduce_only(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for buy_trigger_limit_reduce_only")?,
                price
                    .try_into()
                    .ok()
                    .context("invalid price for buy_trigger_limit_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_trigger_limit_reduce_only")?,
            )
            .await
    }

    /// Trigger-limit sell with `reduce_only`.
    pub async fn sell_trigger_limit_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_trigger_limit_reduce_only(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for sell_trigger_limit_reduce_only")?,
                price
                    .try_into()
                    .ok()
                    .context("invalid price for sell_trigger_limit_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_trigger_limit_reduce_only")?,
            )
            .await
    }

    /// Trigger-market buy with `reduce_only`. Commonly used for stop loss on a short position.
    pub async fn buy_trigger_market_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .buy_trigger_market_reduce_only(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for buy_trigger_market_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for buy_trigger_market_reduce_only")?,
            )
            .await
    }

    /// Trigger-market sell with `reduce_only`. Commonly used for stop loss on a long position.
    pub async fn sell_trigger_market_reduce_only(
        &self,
        symbol: impl AsRef<str>,
        trigger_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<String> {
        self.0
            .sell_trigger_market_reduce_only(
                symbol.as_ref(),
                trigger_price
                    .try_into()
                    .ok()
                    .context("invalid trigger_price for sell_trigger_market_reduce_only")?,
                quantity
                    .try_into()
                    .ok()
                    .context("invalid quantity for sell_trigger_market_reduce_only")?,
            )
            .await
    }

    /// Open a long position with take-profit and stop-loss orders (syntactic sugar).
    ///
    /// Places three orders concurrently:
    /// 1. A **market buy** to open the position.
    /// 2. A **limit sell** at `tp_price` (take-profit, reduce-only).
    /// 3. A **trigger-market sell** at `sl_price` (stop-loss, reduce-only).
    ///
    /// Returns `(master_order_id, take_profit_result, stop_loss_result)`.
    /// Both TP and SL are `reduce_only` and won't flip the position.
    pub async fn buy_tp_sl(
        &self,
        symbol: impl AsRef<str>,
        tp_price: impl TryInto<Decimal>,
        sl_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<(String, anyhow::Result<String>, anyhow::Result<String>)> {
        self.0
            .buy_tp_sl(
                symbol.as_ref(),
                tp_price.try_into().ok().context("invalid tp_price")?,
                sl_price.try_into().ok().context("invalid sl_price")?,
                quantity.try_into().ok().context("invalid quantity")?,
            )
            .await
    }

    /// Open a short position with take-profit and stop-loss orders (syntactic sugar).
    ///
    /// Places three orders concurrently:
    /// 1. A **market sell** to open the position.
    /// 2. A **limit buy** at `tp_price` (take-profit, reduce-only).
    /// 3. A **trigger-market buy** at `sl_price` (stop-loss, reduce-only).
    ///
    /// Returns `(master_order_id, take_profit_result, stop_loss_result)`.
    /// Both TP and SL are `reduce_only` and won't flip the position.
    pub async fn sell_tp_sl(
        &self,
        symbol: impl AsRef<str>,
        tp_price: impl TryInto<Decimal>,
        sl_price: impl TryInto<Decimal>,
        quantity: impl TryInto<Decimal>,
    ) -> anyhow::Result<(String, anyhow::Result<String>, anyhow::Result<String>)> {
        self.0
            .sell_tp_sl(
                symbol.as_ref(),
                tp_price.try_into().ok().context("invalid tp_price")?,
                sl_price.try_into().ok().context("invalid sl_price")?,
                quantity.try_into().ok().context("invalid quantity")?,
            )
            .await
    }
}
