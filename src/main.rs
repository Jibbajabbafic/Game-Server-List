//! Provides a RESTful web server managing Game Servers.
//!
//! API will be:
//!
//! - `GET /api/servers`: return a JSON list of servers.
//! - `POST /api/servers`: create a new server by specifying a name.
//!

use axum::{
    error_handling::HandleErrorLayer,
    extract::{ConnectInfo, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, RwLock},
    time::Duration,
};
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "flappy-server-list=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = Db::default();

    async fn root() -> String {
        String::from("Hello there!")
    }

    // Compose the routes
    let app = Router::new()
        .route("/", get(root))
        .route(
            "/api/servers",
            get(get_servers).post(create_server).delete(remove_server),
        )
        // Add middleware to all routes
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

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

// The query parameters for game server index
#[derive(Debug, Deserialize, Default)]
pub struct Pagination {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
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

#[derive(Debug, Deserialize)]
struct CreateServer {
    name: String,
}

async fn create_server(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(db): State<Db>,
    Json(input): Json<CreateServer>,
) -> impl IntoResponse {
    let server = GameServer {
        id: Uuid::new_v4(),
        name: input.name,
        ip: addr.ip(),
    };

    db.write().unwrap().insert(server.id, server.clone());

    (StatusCode::CREATED, Json(server))
}

#[derive(Debug, Deserialize)]
struct RemoveServer {
    id: Uuid,
}

async fn remove_server(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(db): State<Db>,
    Json(input): Json<RemoveServer>,
) -> impl IntoResponse {
    match db.read().unwrap().get(&input.id) {
        Some(entry) => {
            if entry.ip != addr.ip() {
                tracing::debug!(
                    "request attempted to remove server but IP didn't match! UUID: {} IP: {}",
                    input.id,
                    addr.ip()
                );
                return StatusCode::NOT_FOUND;
            }
        }
        None => return StatusCode::NOT_FOUND,
    }
    db.write().unwrap().remove(&input.id);
    StatusCode::OK
}

type Db = Arc<RwLock<HashMap<Uuid, GameServer>>>;

#[derive(Debug, Serialize, Clone)]
struct GameServer {
    id: Uuid,
    name: String,
    ip: IpAddr,
}
