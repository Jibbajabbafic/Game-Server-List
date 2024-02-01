//! Provides a RESTful web server for listing active game servers.
//!
//! Uses websockets to connect game servers and update their state.
//!
//! API is:
//! - `GET /api/list/servers`: return a JSON list of servers.
//! - `WS /api/list/ws`: connect a game server and update it's state.
//!
//! See README for more details.
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
use game_server_list::{ConnectMessage, GameMessage, GameServer, Pagination, ServerList};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
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
    tracing::info!("env config: {:?}", config);

    // determine server's public ip for local servers
    let server_ip = match public_ip::addr().await {
        Some(ip) => {
            tracing::info!("found server's public ip: {}", ip);
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
        .route("/api/list/ws", get(websocket_handler))
        // determine the secure ip source from the env
        .layer(config.ip_source.into_extension())
        // add default services for error handling, timeout and tracing
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
    tracing::info!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

#[instrument(skip(app_state))]
async fn get_servers(
    pagination: Option<Query<Pagination>>,
    SecureClientIp(ip): SecureClientIp,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("sending server list");
    let Query(pagination) = pagination.unwrap_or_default();
    Json(app_state.server_list.get(&pagination))
}

#[instrument(level = "debug", skip(ws, app_state))]
async fn websocket_handler(
    ws: WebSocketUpgrade,
    SecureClientIp(ip): SecureClientIp,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("new websocket connection");
    ws.protocols(["json"]).on_upgrade(move |socket| {
        handle_socket(socket, ip, app_state.server_list, app_state.server_ip)
    })
}

#[instrument(level = "debug", name = "websocket_handler", skip(socket, server_list))]
async fn handle_socket(
    mut socket: WebSocket,
    ip: IpAddr,
    mut server_list: ServerList,
    server_ip: IpAddr,
) {
    let game_id;

    // wait for the first message with initial server info
    match socket.recv().await {
        Some(result) => match result {
            Ok(msg) => match msg {
                Message::Text(txt) => match parse_connect_message(txt, ip, server_ip) {
                    Ok(server) => {
                        tracing::info!("created new game server: {:?}", server);
                        game_id = server_list.add(server);
                    }
                    Err(e) => {
                        tracing::error!("{:?}", e);
                        return;
                    }
                },
                Message::Close(_) => {
                    tracing::info!("connection closed while waiting for server info");
                    return;
                }
                _ => {
                    tracing::warn!(
                        "got invalid message type while waiting for server info: {:?}",
                        msg
                    );
                    return;
                }
            },
            Err(e) => {
                tracing::error!("error while waiting for server info: {:?}", e);
                return;
            }
        },
        None => {
            tracing::warn!("connection closed unexpectedly while waiting for server info");
            return;
        }
    }
    // begin the main loop to update the game server state
    loop {
        if let Some(msg_type) = socket.recv().await {
            match msg_type {
                Ok(msg) => match msg {
                    Message::Text(t) => parse_game_message(&server_list, &game_id, &t),
                    Message::Close(_) => {
                        tracing::debug!("connection closed");
                        break;
                    }
                    _ => {
                        tracing::warn!("got invalid message type: {:?}", msg)
                    }
                },
                Err(e) => {
                    tracing::error!("error while waiting for game message: {:?}", e);
                    break;
                }
            }
        } else {
            tracing::warn!("connection closed unexpectedly");
            break;
        }
    }
    // Make sure server is always removed if the loop finishes
    remove_server(server_list, &game_id);
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

fn parse_connect_message(txt: String, ip: IpAddr, server_ip: IpAddr) -> Result<GameServer, String> {
    if let Ok(msg) = serde_json::from_str::<ConnectMessage>(&txt) {
        match msg {
            ConnectMessage::V1 { name, port } => {
                tracing::debug!("new game connected with V1 name: {} port: {}", name, port);
                // if this IP is local then it's on the same host so
                // replace the it with the server's public IP
                let mut official = false;
                let ip = if is_local_ipv4(ip) {
                    official = true;
                    server_ip
                } else {
                    ip
                };

                return Ok(GameServer::new(name, ip, false, port, official));
            }
            ConnectMessage::V2 { name, tls, port } => {
                tracing::debug!(
                    "new game connected with V2 name: {} tls: {} port: {}",
                    name,
                    tls,
                    port
                );
                // if this IP is local then it's on the same host so
                // replace the it with the server's public IP
                let mut official = false;
                let ip = if is_local_ipv4(ip) {
                    official = true;
                    server_ip
                } else {
                    ip
                };
                return Ok(GameServer::new(name, ip, tls, port, official));
            }
        }
    }
    Err(format!(
        "failed to parse ConnectMessage from JSON data: {:#?}",
        txt
    ))
}

fn parse_game_message(server_list: &ServerList, server_id: &Uuid, msg: &str) {
    if let Ok(json) = serde_json::from_str::<GameMessage>(msg) {
        match json {
            GameMessage::Status { players } => {
                server_list.update(server_id, |game_server| {
                    game_server.players = players;
                    tracing::info!("updated player count of server: {:?}", game_server);
                });
            }
        }
    } else {
        tracing::error!("failed to parse GameMessage from JSON data: {:#?}", msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn parse_connect_message_v2() {
        let txt = "{\"name\":\"Test's Game\",\"port\":31400,\"tls\":true}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let server_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let expected_server = GameServer::new(
            String::from("Test's Game"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            true,
            31400,
            false,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_v2_reverse_order() {
        let txt = "{\"tls\":true, \"port\":31400, \"name\":\"Test's Game\"}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let server_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let expected_server = GameServer::new(
            String::from("Test's Game"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            true,
            31400,
            false,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_v2_official() {
        let txt = "{\"name\":\"Another Game\",\"port\":65535,\"tls\":true}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 123));
        let server_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 123));
        let expected_server = GameServer::new(
            String::from("Another Game"),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 123)),
            true,
            65535,
            true,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_v1() {
        let txt = "{\"name\":\"Test\",\"port\":12345}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let server_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let expected_server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_v1_reverse_order() {
        let txt = "{\"port\":12345, \"name\":\"Test\"}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let server_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let expected_server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_v1_official() {
        let txt = "{\"name\":\"Test\",\"port\":12345}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(172, 16, 0, 22));
        let server_ip = IpAddr::V4(Ipv4Addr::new(172, 16, 0, 22));
        let expected_server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(172, 16, 0, 22)),
            false,
            12345,
            true,
        );
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert_eq!(result, Ok(expected_server));
    }

    #[test]
    fn parse_connect_message_unknown() {
        let txt = "{\"wasd\":\"Test\",\"port\":12345,\"asdoasdoaisd\":59912}".to_string();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let server_ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let result: Result<GameServer, String> = parse_connect_message(txt, ip, server_ip);
        assert!(result.is_err());
    }
}
