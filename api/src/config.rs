use dotenvy::dotenv;
use std::env;

#[derive(Debug, Clone, Copy)]
pub struct GrpcPort(pub u16);

#[derive(Debug, Clone, Copy)]
pub struct RestPort(pub u16);

impl GrpcPort {
    pub fn socket_addr(self) -> std::net::SocketAddr {
        ([0, 0, 0, 0], self.0).into()
    }
}

impl RestPort {
    pub fn socket_addr(self) -> std::net::SocketAddr {
        ([0, 0, 0, 0], self.0).into()
    }
}

#[derive(Debug)]
pub struct Config {
    pub state_db_path: String,
    pub grpc_port: GrpcPort,
    pub rest_port: RestPort,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv().ok();

        let state_db_path = env::var("STATE_DB_PATH")
            .unwrap_or_else(|_| "./data/state".to_string());

        let grpc_port = env::var("GRPC_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50051u16);

        let rest_port = env::var("REST_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000u16);

        Config {
            state_db_path,
            grpc_port: GrpcPort(grpc_port),
            rest_port: RestPort(rest_port),
        }
    }
}
