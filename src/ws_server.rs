//! Example websocket server.
//!
//! Run with
//!
//! ```not_rust
//! cd examples && cargo run -p example-websockets
//! ```

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, TypedHeader,
    },
    headers,
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, str};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "godot-server-list=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // build our application with some routes
    let app = Router::new()
        // websocket route
        .route("/ws", get(ws_handler))
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // run the server
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> impl IntoResponse {
    println!("connection from IP: {}", addr.ip());
    if let Some(TypedHeader(user_agent)) = user_agent {
        println!("user agent: `{}`", user_agent.as_str());
    }

    ws.protocols(["json"]).on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    loop {
        if let Some(msg) = socket.recv().await {
            if let Ok(msg) = msg {
                match msg {
                    Message::Text(t) => {
                        println!("client sent str: {:?}", t);
                    }
                    Message::Binary(bin) => {
                        if let Ok(string) = str::from_utf8(&bin) {
                            parse_game_message(string);
                        } else {
                            println!("client sent unknown binary data: {:?}", bin);
                        }
                    }
                    Message::Ping(_) => {
                        println!("socket ping");
                    }
                    Message::Pong(_) => {
                        println!("socket pong");
                    }
                    Message::Close(_) => {
                        println!("client disconnected");
                        return;
                    }
                }
            } else {
                println!("client disconnected");
                return;
            }
        }
    }
}

fn parse_game_message(msg: &str) {
    if let Ok(json) = serde_json::from_str::<GameMessage>(msg) {
        match json {
            GameMessage::Connect { name } => {
                println!("new game connected with name: {}", name)
            }
            GameMessage::Status { players } => {
                println!("status update with players: {}", players)
            }
        }
    } else {
        println!("Got unknown JSON data: {:#?}", msg);
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GameMessage {
    Connect { name: String },
    Status { players: usize },
}
