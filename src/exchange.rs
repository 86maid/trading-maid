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

    async fn get_history_position_list(&self, symbol: &str) -> anyhow::Result<Vec<HistoryPosition>>;

    async fn append_position_margin(&self, symbol: &str, margin: f64) -> anyhow::Result<()>;

    async fn get_equity(&self) -> anyhow::Result<f64>;

    async fn get_cash(&self) -> anyhow::Result<f64>;

    async fn get_leverage(&self, symbol: &str) -> anyhow::Result<u32>;

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> anyhow::Result<()>;

    async fn get_metadata(&self, symbol: &str) -> anyhow::Result<Metadata>;

    async fn buy(&self, symbol: &str, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: 0.0,
            price: 0.0,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn sell(&self, symbol: &str, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: 0.0,
            price: 0.0,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn buy_limit(&self, symbol: &str, price: f64, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: 0.0,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn sell_limit(&self, symbol: &str, price: f64, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: 0.0,
            price,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn buy_trigger_limit(
        &self,
        symbol: &str,
        trigger_price: f64,
        price: f64,
        quantity: f64,
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
        trigger_price: f64,
        price: f64,
        quantity: f64,
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
        trigger_price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price: 0.0,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn sell_trigger_market(
        &self,
        symbol: &str,
        trigger_price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price: 0.0,
            quantity,
            reduce_only: false,
        })
        .await
    }

    async fn buy_reduce_only(&self, symbol: &str, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: 0.0,
            price: 0.0,
            quantity,
            reduce_only: true,
        })
        .await
    }

    async fn sell_reduce_only(&self, symbol: &str, quantity: f64) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: 0.0,
            price: 0.0,
            quantity,
            reduce_only: true,
        })
        .await
    }

    async fn buy_limit_reduce_only(
        &self,
        symbol: &str,
        price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price: 0.0,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    async fn sell_limit_reduce_only(
        &self,
        symbol: &str,
        price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price: 0.0,
            price,
            quantity,
            reduce_only: true,
        })
        .await
    }

    async fn buy_trigger_limit_reduce_only(
        &self,
        symbol: &str,
        trigger_price: f64,
        price: f64,
        quantity: f64,
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
        trigger_price: f64,
        price: f64,
        quantity: f64,
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
        trigger_price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Buy,
            trigger_price,
            price: 0.0,
            quantity,
            reduce_only: true,
        })
        .await
    }

    async fn sell_trigger_market_reduce_only(
        &self,
        symbol: &str,
        trigger_price: f64,
        quantity: f64,
    ) -> anyhow::Result<String> {
        self.place_order(Order {
            symbol: symbol.to_string(),
            side: Side::Sell,
            trigger_price,
            price: 0.0,
            quantity,
            reduce_only: true,
        })
        .await
    }
}
