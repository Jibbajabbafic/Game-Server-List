//! Provides a RESTful web server managing Game Servers.
//!
//! Uses websockets to connect game servers and update their state.
//!
//! API will be:
//!
//! - `GET /api/servers`: return a JSON list of servers.
//! - `WS /api/servers/ws`: connect a game server and update it's state.
//!
//! Run with
//!
//! ```not_rust
//! cargo run
//! ```

use axum::{
    error_handling::HandleErrorLayer,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    net::SocketAddr,
    str,
    sync::{Arc, RwLock},
    time::Duration,
};
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "godot_server_list=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = Db::default();

    // build our application with some routes
    let app = Router::new()
        .route("/api/servers", get(get_servers))
        // websocket route
        .route("/api/servers/ws", get(ws_handler))
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        // add fallback option
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|error: BoxError| async move {
                    if error.is::<tower::timeout::error::Elapsed>() {
                        Ok(StatusCode::REQUEST_TIMEOUT)
                    } else {
                        Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unhandled internal error: {}", error),
                        ))
                    }
                }))
                .timeout(Duration::from_secs(10))
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        )
        .with_state(db);

    // run the server
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn get_servers(
    pagination: Option<Query<Pagination>>,
    State(db): State<Db>,
) -> impl IntoResponse {
    let servers = db.read().unwrap();

    let Query(pagination) = pagination.unwrap_or_default();

    let servers = servers
        .values()
        .skip(pagination.offset.unwrap_or(0))
        .take(pagination.limit.unwrap_or(usize::MAX))
        .cloned()
        .collect::<Vec<_>>();

    Json(servers)
}

// The query parameters for game server index
#[derive(Debug, Deserialize, Default)]
pub struct Pagination {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(db): State<Db>,
) -> impl IntoResponse {
    tracing::debug!("connection from IP: {}", addr.ip());
    ws.protocols(["json"])
        .on_upgrade(move |socket| handle_socket(socket, addr, db))
}

async fn handle_socket(mut socket: WebSocket, addr: SocketAddr, db: Db) {
    let game_id = Uuid::new_v4();
    // loop until the first message is received, which should be the name
    while let Some(Ok(Message::Text(msg))) = socket.recv().await {
        if let Ok(GameMessage::Connect { name }) = serde_json::from_str::<GameMessage>(&msg) {
            let server = GameServer {
                id: game_id,
                name,
                ip: addr.ip(),
                players: 0,
            };
            tracing::info!("created new game server: {:?}", server);
            db.write().unwrap().insert(server.id, server);
            break;
        } else {
            tracing::error!("got unknown JSON data: {:#?}", msg);
        }
    }

    // begin the main loop to update the game server state
    loop {
        if let Some(msg) = socket.recv().await {
            if let Ok(msg) = msg {
                match msg {
                    Message::Text(t) => {
                        parse_game_message(&db, game_id, &t);
                    }
                    Message::Close(_) => {
                        remove_server(db, game_id);
                        return;
                    }
                    _ => {
                        tracing::warn!("got invalid message type: {:?}", msg)
                    }
                }
            } else {
                remove_server(db, game_id);
                return;
            }
        }
    }
}

fn remove_server(db: Db, id: Uuid) {
    tracing::info!("client disconnected");
    db.write().unwrap().remove(&id);
}

fn parse_game_message(db: &Db, id: Uuid, msg: &str) {
    if let Ok(json) = serde_json::from_str::<GameMessage>(msg) {
        match json {
            GameMessage::Connect { name } => {
                tracing::info!("new game connected with name: {}", name)
            }
            GameMessage::Status { players } => {
                db.write().unwrap().entry(id).and_modify(|game_server| {
                    game_server.players = players;
                    tracing::info!("updated player count of server: {:?}", game_server);
                });
            }
        }
    } else {
        tracing::error!("got unknown JSON data: {:#?}", msg);
    }
}

type Db = Arc<RwLock<HashMap<Uuid, GameServer>>>;

#[derive(Debug, Serialize, Clone)]
struct GameServer {
    id: Uuid,
    name: String,
    ip: IpAddr,
    players: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GameMessage {
    Connect { name: String },
    Status { players: u32 },
}
