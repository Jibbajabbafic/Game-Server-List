use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, RwLock},
};
use uuid::Uuid;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct GameServer {
    name: String,
    ip: IpAddr,
    tls: bool,
    port: u16,
    official: bool,
    pub players: u32,
}

impl GameServer {
    pub fn new(name: String, ip: IpAddr, tls: bool, port: u16, official: bool) -> GameServer {
        GameServer {
            name,
            ip,
            tls,
            port,
            official,
            players: 0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GameMessage {
    Connect { name: String, tls: bool, port: u16 },
    Status { players: u32 },
}

#[derive(Clone)]
pub struct ServerList {
    servers: Arc<RwLock<HashMap<Uuid, GameServer>>>,
}

impl ServerList {
    pub fn new() -> ServerList {
        ServerList {
            servers: Arc::default(),
        }
    }
    pub fn add(&mut self, server: GameServer) -> Uuid {
        let mut servers = self.servers.write().unwrap();
        let mut server_id = Uuid::new_v4();
        // just in case the UUIDv4 clashes with an existing one
        loop {
            if servers.contains_key(&server_id) {
                server_id = Uuid::new_v4();
            } else {
                break;
            }
        }
        servers.insert(server_id, server);
        server_id
    }
    pub fn remove(&mut self, server_id: &Uuid) -> Option<GameServer> {
        let mut servers = self.servers.write().unwrap();
        servers.remove(server_id)
    }
    pub fn len(&self) -> usize {
        let servers = self.servers.read().unwrap();
        servers.len()
    }
    pub fn is_empty(&self) -> bool {
        let servers = self.servers.read().unwrap();
        servers.is_empty()
    }
    pub fn get(&self, pagination: &Pagination) -> Vec<GameServer> {
        let servers = self.servers.read().unwrap();
        servers
            .values()
            .skip(pagination.offset.unwrap_or(0))
            .take(pagination.limit.unwrap_or(usize::MAX))
            .cloned()
            .collect::<Vec<_>>()
    }
    pub fn update<F: FnOnce(&mut GameServer)>(&self, server_id: &Uuid, func: F) {
        self.servers
            .write()
            .unwrap()
            .entry(*server_id)
            .and_modify(func);
    }
}

impl Default for ServerList {
    fn default() -> Self {
        Self::new()
    }
}

// The query parameters for game server index
#[derive(Debug, Deserialize, Default)]
pub struct Pagination {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn add_server() {
        let server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let mut server_list = ServerList::new();
        assert_eq!(server_list.len(), 0);
        server_list.add(server);
        assert_eq!(server_list.len(), 1);
    }

    #[test]
    fn remove_server() {
        let server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let mut server_list = ServerList::new();
        let uuid = server_list.add(server);
        assert_eq!(server_list.len(), 1);
        server_list.remove(&uuid);
        assert_eq!(server_list.len(), 0);
    }

    #[test]
    fn get_server() {
        let server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let expected = server.clone();
        let mut server_list = ServerList::new();
        server_list.add(server);
        let pagination = Pagination::default();
        let servers = server_list.get(&pagination);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0], expected);
    }
    #[test]
    fn update_server() {
        let server = GameServer::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            false,
            12345,
            false,
        );
        let mut server_list = ServerList::new();
        let server_id = server_list.add(server);
        server_list.update(&server_id, |game_server| game_server.players = 10);
        let pagination = Pagination::default();
        let updated_server = server_list.get(&pagination);
        assert_eq!(updated_server[0].players, 10)
    }
}
