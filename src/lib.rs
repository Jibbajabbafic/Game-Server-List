use std::{collections::HashMap, net::IpAddr};
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ServerList {
    servers: HashMap<Uuid, ServerEntry>,
}

impl ServerList {
    pub fn new() -> ServerList {
        ServerList {
            servers: HashMap::new(),
        }
    }
    pub fn add(&mut self, server: ServerEntry) -> Uuid {
        let mut server_uuid = Uuid::new_v4();
        // Just in case the UUIDv4 clashes with an existing one
        loop {
            if self.servers.contains_key(&server_uuid) {
                server_uuid = Uuid::new_v4();
            } else {
                break;
            }
        }
        self.servers.insert(server_uuid, server);
        server_uuid
    }
    pub fn remove(&mut self, uuid: &Uuid) -> Option<ServerEntry> {
        self.servers.remove(uuid)
    }
    pub fn len(&self) -> usize {
        self.servers.len()
    }
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

impl Default for ServerList {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ServerEntry {
    name: String,
    ip: IpAddr,
    players: u8,
}

impl ServerEntry {
    pub fn new(name: String, ip: IpAddr) -> ServerEntry {
        ServerEntry {
            name,
            ip,
            players: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn add_server() {
        let server = ServerEntry::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );
        let mut server_list = ServerList::new();
        assert_eq!(server_list.len(), 0);
        server_list.add(server);
        assert_eq!(server_list.len(), 1);
    }

    #[test]
    fn remove_server() {
        let server = ServerEntry::new(
            String::from("Test"),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );
        let mut server_list = ServerList::new();
        let uuid = server_list.add(server);
        assert_eq!(server_list.len(), 1);
        server_list.remove(&uuid);
        assert_eq!(server_list.len(), 0);
    }
}
