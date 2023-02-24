FROM rust:1.65 as builder
WORKDIR /usr/src/godot_server_list
COPY . .
RUN cargo install --path .

FROM debian:10.13-slim
# RUN apt-get update && apt-get install -y extra-runtime-dependencies && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/godot_server_list /usr/local/bin/godot_server_list
CMD ["godot_server_list"]