use std::sync::Arc;

use anyhow::Context;
use rust_decimal::Decimal;

use crate::data::*;
use crate::order::*;

#[async_trait::async_trait]
pub trait Exchange
where
    Self: Send + Sync + 'static,
{
    async fn next(&self, symbol: &str, level: Level) -> anyhow::Result<Option<KLine>>;

    async fn place_order(&self, order: Order) -> anyhow::Result<String>;

    async fn cancel_order(&self, symbol: &str, id: &str) -> anyhow::Result<()>;

    async fn cancel_all_order(&self, symbol: &str) -> anyhow::Result<()>;

    async fn get_order(&self, id: &str) -> anyhow::Result<Option<OrderMessage>>;

    async fn get_history_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>>;

    async fn get_pending_order_list(&self, symbol: &str) -> anyhow::Result<Vec<OrderMessage>>;

    async fn get_position(&self, symbol: &str) -> anyhow::Result<Option<Position>>;

    async fn close_all_position(&self, symbol: &str) -> anyhow::Result<()>;

    async fn get_history_position_list(&self, symbol: &str)
    -> anyhow::Result<Vec<HistoryPosition>>;

    async fn append_position_margin(&self, symbol: &str, margin: Decimal) -> anyhow::Result<()>;

    async fn get_equity(&self) -> anyhow::Result<Decimal>;

    async fn get_cash(&self) -> anyhow::Result<Decimal>;

    async fn get_leverage(&self, symbol: &str) -> anyhow::Result<u32>;

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()>;

    async fn get_metadata(&self, symbol: &str) -> anyhow::Result<Metadata>;

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
}

#[derive(Clone)]
pub struct ExchangeWrapper(Arc<dyn Exchange + 'static>);

impl ExchangeWrapper {
    pub fn new(exchange: Arc<dyn Exchange + 'static>) -> Self {
        Self(exchange)
    }

    pub(crate) async fn next(&self, symbol: &str, level: Level) -> anyhow::Result<Option<KLine>> {
        self.0.next(symbol, level).await
    }

    pub async fn place_order(&self, order: Order) -> anyhow::Result<String> {
        self.0.place_order(order).await
    }

    pub async fn cancel_order(
        &self,
        symbol: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        self.0.cancel_order(symbol.as_ref(), id.as_ref()).await
    }

    pub async fn cancel_all_order(&self, symbol: impl AsRef<str>) -> anyhow::Result<()> {
        self.0.cancel_all_order(symbol.as_ref()).await
    }

    pub async fn get_order(&self, id: impl AsRef<str>) -> anyhow::Result<Option<OrderMessage>> {
        self.0.get_order(id.as_ref()).await
    }

    pub async fn get_history_order_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<OrderMessage>> {
        self.0.get_history_order_list(symbol.as_ref()).await
    }

    pub async fn get_pending_order_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<OrderMessage>> {
        self.0.get_pending_order_list(symbol.as_ref()).await
    }

    pub async fn get_position(&self, symbol: impl AsRef<str>) -> anyhow::Result<Option<Position>> {
        self.0.get_position(symbol.as_ref()).await
    }

    pub async fn close_all_position(&self, symbol: impl AsRef<str>) -> anyhow::Result<()> {
        self.0.close_all_position(symbol.as_ref()).await
    }

    pub async fn get_history_position_list(
        &self,
        symbol: impl AsRef<str>,
    ) -> anyhow::Result<Vec<HistoryPosition>> {
        self.0.get_history_position_list(symbol.as_ref()).await
    }

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

    pub async fn get_equity(&self) -> anyhow::Result<Decimal> {
        self.0.get_equity().await
    }

    pub async fn get_cash(&self) -> anyhow::Result<Decimal> {
        self.0.get_cash().await
    }

    pub async fn get_leverage(&self, symbol: impl AsRef<str>) -> anyhow::Result<u32> {
        self.0.get_leverage(symbol.as_ref()).await
    }

    pub async fn set_leverage(&self, symbol: impl AsRef<str>, leverage: u32) -> anyhow::Result<()> {
        self.0.set_leverage(symbol.as_ref(), leverage).await
    }

    pub async fn get_metadata(&self, symbol: impl AsRef<str>) -> anyhow::Result<Metadata> {
        self.0.get_metadata(symbol.as_ref()).await
    }

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
}
