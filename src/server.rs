use crate::data::DataSource;
use crate::order::{HistoryPosition, OrderMessage};
use crate::util::to_html;
use anyhow::Context;
use reqwest::StatusCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Notify;
use warp::Filter;
use warp::reply::Reply;

struct ServerHandle {
    port: u16,
    state_json: Arc<Mutex<String>>,
    last_heartbeat: Arc<Mutex<Option<Instant>>>,
}

fn make_state_json(
    data_source: impl AsRef<[DataSource]>,
    history_position: impl AsRef<[HistoryPosition]>,
    history_order: impl AsRef<[OrderMessage]>,
) -> anyhow::Result<String> {
    Ok(serde_json::json!({
        "dataSourceList": data_source.as_ref(),
        "historyPositionList": history_position.as_ref(),
        "historyOrderList": history_order.as_ref(),
    })
    .to_string())
}

fn start_server(
    data_source: impl AsRef<[DataSource]>,
    history_position: impl AsRef<[HistoryPosition]>,
    history_order: impl AsRef<[OrderMessage]>,
) -> anyhow::Result<ServerHandle> {
    let hash = data_source
        .as_ref()
        .iter()
        .map(|v| {
            v.metadata.symbol.clone()
                + "["
                + v.data
                    .first()
                    .map(|v| v.time)
                    .unwrap_or_default()
                    .to_string()
                    .as_str()
                + ","
                + v.data
                    .last()
                    .map(|v| v.time)
                    .unwrap_or_default()
                    .to_string()
                    .as_str()
                + "]"
        })
        .collect::<Vec<_>>()
        .join(",");

    Arc::new(data_source.as_ref());

    let script_data_source = Arc::new(format!(
        "<script>window.dataSourceList={}</script>",
        serde_json::to_string(data_source.as_ref())?
    ));

    let script_trading = Arc::new(format!(
        "<script>window.historyPositionList={};window.historyOrderList={}</script>",
        serde_json::to_string(history_position.as_ref())?,
        serde_json::to_string(history_order.as_ref())?
    ));

    let html = Arc::new(to_html(data_source, history_position, history_order));

    let root = warp::path::end().and_then(move || {
        let html = html.clone();
        async move { Ok::<_, warp::Rejection>(warp::reply::html((*html).clone())) }
    });

    let update = warp::path!("update" / String).and_then(move |client_hash: String| {
        let hash = hash.clone();
        let script_data_source = script_data_source.clone();
        let script_trading = script_trading.clone();

        async move {
            let script_data_source = (*script_data_source).clone();
            let script_trading = (*script_trading).clone();
            let script = format!("{}{}", script_data_source, script_trading);

            if client_hash == *hash {
                Ok::<Box<dyn Reply>, warp::Rejection>(Box::new(warp::reply::with_status(
                    "",
                    StatusCode::NOT_MODIFIED,
                )))
            } else {
                Ok::<Box<dyn Reply>, warp::Rejection>(Box::new(warp::reply::with_header(
                    script,
                    "Content-Type",
                    "application/javascript; charset=utf-8",
                )))
            }
        }
    });

    let route = root.or(update);

    warp::serve(route).bind(([127, 0, 0, 1], 0));

    todo!()
}

pub async fn open_in_server(
    data_source: impl Into<Vec<DataSource>>,
    history_position: impl Into<Vec<HistoryPosition>>,
    history_order: impl Into<Vec<OrderMessage>>,
) -> anyhow::Result<()> {
    let data_source: Arc<[DataSource]> = Arc::from(data_source.into());
    let history_position: Arc<[HistoryPosition]> = Arc::from(history_position.into());
    let history_order: Arc<[OrderMessage]> = Arc::from(history_order.into());

    let hash = data_source
        .as_ref()
        .iter()
        .map(|v| {
            v.metadata.symbol.clone()
                + "["
                + v.data
                    .first()
                    .map(|v| v.time)
                    .unwrap_or_default()
                    .to_string()
                    .as_str()
                + ","
                + v.data
                    .last()
                    .map(|v| v.time)
                    .unwrap_or_default()
                    .to_string()
                    .as_str()
                + "]"
        })
        .collect::<Vec<_>>()
        .join(",");

    let state = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let notify = Arc::new(Notify::new());

    let root = warp::path::end().and_then({
        let hash = hash.clone();
        let state = state.clone();
        let data_source = data_source.clone();
        let history_position = history_position.clone();
        let history_order = history_order.clone();
        let notify = notify.clone();

        move || {
            let hash = hash.clone();
            let state = state.clone();
            let data_source = data_source.clone();
            let history_position = history_position.clone();
            let history_order = history_order.clone();

            notify.notify_one();

            let text = format!(
                "<script>window.hash={};window.state={};window.dataSourceList={};window.historyPositionList={};window.historyOrderList={}</script>",
                &serde_json::to_string(&hash).unwrap(),
                &serde_json::to_string(&state).unwrap(),
                &serde_json::to_string(data_source.as_ref()).unwrap(),
                &serde_json::to_string(history_position.as_ref()).unwrap(),
                &serde_json::to_string(history_order.as_ref()).unwrap(),
            );

            let html = include_str!("../web/dist/index.html").replace("<!-- template -->", &text);

            async move {
                Ok::<_, warp::Rejection>(warp::reply::html(html))
            }
        }
    });

    let update = warp::path!("update" / String / u64).and_then({
        let notify = notify.clone();

        move |client_hash: String, client_state: u64| {
            let hash = hash.clone();
            let data_source = data_source.clone();
            let history_position = history_position.clone();
            let history_order = history_order.clone();

            notify.notify_one();

            async move {
                if client_hash == *hash {
                    if client_state != state {
                        Ok::<Box<dyn Reply>, warp::Rejection>(Box::new(warp::reply::with_header(
                            format!(
                                "window.historyPositionList={};window.historyOrderList={}",
                                serde_json::to_string(history_position.as_ref()).unwrap(),
                                serde_json::to_string(history_order.as_ref()).unwrap()
                            ),
                            "Content-Type",
                            "application/javascript; charset=utf-8",
                        )))
                    } else {
                        Ok::<Box<dyn Reply>, warp::Rejection>(Box::new(warp::reply::with_status(
                            "",
                            StatusCode::NOT_MODIFIED,
                        )))
                    }
                } else {
                    Ok::<Box<dyn Reply>, warp::Rejection>(Box::new(warp::reply::with_header(
                        to_html(data_source, history_position, history_order),
                        "Content-Type",
                        "text/html; charset=utf-8",
                    )))
                }
            }
        }
    });

    let route = root.or(update).with(warp::cors().allow_any_origin());

    fn is_port_in_use(port: u16) -> bool {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
    }

    let port = (8686..9999)
        .find(|&port| !is_port_in_use(port))
        .context("no available port found")?;

    tokio::spawn(warp::serve(route).bind(([0, 0, 0, 0], port)).await.run());

    if tokio::time::timeout(Duration::from_secs(1), notify.notified())
        .await
        .is_err()
    {
        webbrowser::open(&format!("http://127.0.0.1:{}", port))?;
        notify.notified().await;
    }

    Ok(())
}
