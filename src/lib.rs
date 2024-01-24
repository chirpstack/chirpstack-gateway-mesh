#[macro_use]
extern crate anyhow;

use tokio::fs::remove_file;

pub mod backend;
pub mod cmd;
pub mod config;
pub mod helpers;
pub mod logging;
pub mod packets;
pub mod proxy;
pub mod relay;

pub async fn cleanup_socket_file(path: &str) {
    let ep = match path.parse::<zeromq::Endpoint>() {
        Ok(v) => v,
        Err(_) => {
            return;
        }
    };

    if let zeromq::Endpoint::Ipc(Some(path)) = ep {
        let _ = remove_file(path).await;
    }
}
