# Game Server List - Generic Server Browser API

This repo contains a generic game server list written in Rust which can be used as an API for
in-game server browsers. Uses WebSockets to connect new game servers and update their state in real
time. This should work with any game engine as long as it's able to send HTTP requests and start
WebSocket connections.

Originally written for the open source game
[Flappy Race](https://github.com/Jibby-Games/Flappy-Race).

## Features
- Automatically detects IP addresses of new game servers to stop them spoofing their IP.
- Automatically flags game servers originating from the same IP as the Game Server List as
"official" ones (can be useful on the game client).
- Unit tested
- Stores the following info for each game server:
  - Name: String
  - IP: IpAddr
  - TLS: bool
  - Port: u16
  - Official: bool
  - Players: u32 (updated in real time using messages from the game server)

## API Overview
- `GET /api/list/servers`: return a JSON list of active servers.
- `WebSocket /api/list/ws`: used to connect new game servers and update their state.
  - Must use text mode for messages
  - Must send some initial info to create an entry for the server
  - Can send more payloads to update game stats
  - See below for more details.

## Client Usage in Godot
This is how to use the API with Godot, but should work similarly for other game engines. Expects
messages to be sent in JSON format.
A fully working component that implements this (including automatic reconnection) can be found in
the [Flappy Race repo here](https://github.com/Jibby-Games/Flappy-Race/blob/main/server/server_list_handler.gd).

### 1. Starting the WebSocket Connection
```py
    # Create WebSocket client and connect from the game server
    var client = WebSocketClient.new()
    var url = <URL to your server list>
    var result = client.connect_to_url(url, ["json"], false)
    assert(result == OK)
    # IMPORTANT: Must use text mode for the WebSocket!
    client.get_peer(1).set_write_mode(WebSocketPeer.WRITE_MODE_TEXT)

    # Send initial info to the server list so it can create an entry
    var game_info := {"name": game_name, "tls": use_tls, "port": game_port}
	result := client.get_peer(1).put_packet(to_json(game_info).to_utf8())
	assert(result == OK)
```

### 2. Updating Game Stats
```py
    # This will update the player count in the server list for the current game
    var game_stats = {"players": value}
    var result := client.get_peer(1).put_packet(to_json(game_stats).to_utf8())
    assert(result == OK)
```
## Running the Server List
Can either be compiled and run standalone or through the Docker images provided on Dockerhub at
[`jibby/game-server-list`](https://hub.docker.com/repository/docker/jibby/game-server-list/general).
### Docker
Docker images are automatically built from the `main` branch for every commit.
Ensure you have Docker installed and then run:
```
docker pull jibby/game-server-list:latest
docker run jibby/game-server-list
```
See [this page](https://hub.docker.com/repository/docker/jibby/game-server-list/general) for more
details on available versions.
### Standalone
First ensure you have Rust and cargo installed on your system and then use:
```bash
cargo run
```
