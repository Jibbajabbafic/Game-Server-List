//! Provides a RESTful web server for listing active game servers.
//!
//! Uses websockets to connect game servers and update their state.
//!
//! API will be:
//!
//! - `GET /api/list/servers`: return a JSON list of servers.
//! - `WS /api/list/ws`: connect a game server and update it's state.
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
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use axum_client_ip::{SecureClientIp, SecureClientIpSource};
use game_server_list::{GameMessage, GameServer, Pagination, ServerList};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::instrument;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

// env config with defaults
#[derive(serde::Deserialize, Debug)]
struct Config {
    #[serde(default = "default_ip_source")]
    ip_source: SecureClientIpSource,
}

fn default_ip_source() -> SecureClientIpSource {
    SecureClientIpSource::ConnectInfo
}

// shared app state
#[derive(Clone)]
struct AppState {
    server_list: ServerList,
    server_ip: IpAddr,
}

#[tokio::main]
async fn main() {
    // enable logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "game_server_list=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // get config from env
    let config: Config = envy::from_env().unwrap();
    tracing::debug!("env config: {:?}", config);

    // determine server's public ip for local servers
    let server_ip = match public_ip::addr().await {
        Some(ip) => {
            tracing::debug!("found server's public ip: {}", ip);
            ip
        }
        None => panic!("unable to find server's public ip address, please make sure it has a connection to the internet"),
    };

    let app_state = AppState {
        server_list: ServerList::new(),
        server_ip,
    };

    // build our application with some routes
    let app = Router::new()
        .route("/api/list/servers", get(get_servers))
        // websocket route
        .route("/api/list/ws", get(ws_handler))
        // determine the secure ip source from the env
        .layer(config.ip_source.into_extension())
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
        .with_state(app_state);

    // run the server
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn get_servers(
    pagination: Option<Query<Pagination>>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    let Query(pagination) = pagination.unwrap_or_default();
    Json(app_state.server_list.get(&pagination))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    SecureClientIp(ip): SecureClientIp,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    tracing::debug!("new connection from: {}", ip);
    ws.protocols(["json"]).on_upgrade(move |socket| {
        handle_socket(socket, ip, app_state.server_list, app_state.server_ip)
    })
}

#[instrument(level = "debug", name = "socket_handler", skip(socket, server_list))]
async fn handle_socket(
    mut socket: WebSocket,
    ip: IpAddr,
    mut server_list: ServerList,
    server_ip: IpAddr,
) {
    let mut game_id = Uuid::nil();

    // loop until the first message is received, which should be the name
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(txt) => {
                if let Ok(GameMessage::Connect { name, port }) =
                    serde_json::from_str::<GameMessage>(&txt)
                {
                    // if this IP is local then it's on the same host so
                    // replace the it with the server's public IP
                    let ip = if is_local_ipv4(ip) { server_ip } else { ip };

                    let server = GameServer::new(name, ip, port);
                    tracing::info!("created new game server: {:?}", server);
                    game_id = server_list.add(server);
                    break;
                } else {
                    tracing::error!("got unknown JSON data: {:?}", txt);
                }
            }
            Message::Close(_) => {
                tracing::debug!("connection closed: {}", ip);
                return;
            }
            _ => {
                tracing::warn!("got invalid message type: {:?}", msg)
            }
        }
    }
    // don't allow the game_id to change after this
    let game_id = game_id;
    // begin the main loop to update the game server state
    loop {
        if let Some(msg) = socket.recv().await {
            if let Ok(msg) = msg {
                match msg {
                    Message::Text(t) => {
                        parse_game_message(&server_list, &game_id, &t);
                    }
                    Message::Close(_) => {
                        tracing::debug!("connection closed: {}", ip);
                        remove_server(server_list, &game_id);
                        return;
                    }
                    _ => {
                        tracing::warn!("got invalid message type: {:?}", msg)
                    }
                }
            } else {
                tracing::debug!("unexpected error: {:?}", msg);
                remove_server(server_list, &game_id);
                return;
            }
        }
    }
}

fn is_local_ipv4(ip: IpAddr) -> bool {
    if let IpAddr::V4(ipv4) = ip {
        return ipv4.is_private();
    }
    return false;
}

fn remove_server(mut server_list: ServerList, game_id: &Uuid) {
    match server_list.remove(game_id) {
        Some(entry) => tracing::info!("deleted game server: {:?}", entry),
        None => tracing::error!("failed to remove game server with id: {:?}", game_id),
    }
}

fn parse_game_message(server_list: &ServerList, server_id: &Uuid, msg: &str) {
    if let Ok(json) = serde_json::from_str::<GameMessage>(msg) {
        match json {
            GameMessage::Connect { name, port } => {
                tracing::info!("new game connected with name: {} port: {}", name, port)
            }
            GameMessage::Status { players } => {
                server_list.update(server_id, |game_server| {
                    game_server.players = players;
                    tracing::info!("updated player count of server: {:?}", game_server);
                });
            }
        }
    } else {
        tracing::error!("got unknown JSON data: {:#?}", msg);
    }
}
