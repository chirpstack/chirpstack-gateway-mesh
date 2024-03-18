use std::time::Duration;

use once_cell::sync::OnceCell;
use tokio::fs::remove_file;
use tokio::sync::Mutex;
use tokio::time::sleep;
use zeromq::{Socket, SocketRecv, SocketSend};

use chirpstack_gateway_relay::config::{self, Configuration};

pub static FORWARDER_EVENT_SOCK: OnceCell<Mutex<zeromq::SubSocket>> = OnceCell::new();
pub static FORWARDER_COMMAND_SOCK: OnceCell<Mutex<zeromq::ReqSocket>> = OnceCell::new();

pub static BACKEND_EVENT_SOCK: OnceCell<Mutex<zeromq::PubSocket>> = OnceCell::new();
pub static BACKEND_COMMAND_SOCK: OnceCell<Mutex<zeromq::RepSocket>> = OnceCell::new();

pub static RELAY_BACKEND_EVENT_SOCK: OnceCell<Mutex<zeromq::PubSocket>> = OnceCell::new();
pub static RELAY_BACKEND_COMMAND_SOCK: OnceCell<Mutex<zeromq::RepSocket>> = OnceCell::new();

pub async fn setup(border_gateway: bool) {
    let conf = get_config(border_gateway);
    let _ = config::set(conf);
    init_backend(border_gateway).await;
    init_relay().await;
    init_forwarder(border_gateway).await;
}

pub fn get_config(border_gateway: bool) -> Configuration {
    Configuration {
        relay: config::Relay {
            border_gateway,
            frequencies: vec![868100000],
            data_rate: config::DataRate {
                modulation: config::Modulation::LORA,
                spreading_factor: 7,
                bandwidth: 125000,
                code_rate: Some(config::CodeRate::Cr45),
                ..Default::default()
            },
            tx_power: 16,
            proxy_api: config::ProxyApi {
                event_bind: "ipc:///tmp/gateway_relay_event".into(),
                command_bind: "ipc:///tmp/gateway_relay_command".into(),
            },
            ..Default::default()
        },
        backend: config::Backend {
            concentratord: config::Concentratord {
                event_url: "ipc:///tmp/concentratord_event".into(),
                command_url: "ipc:///tmp/concentratord_command".into(),
            },
            relay_concentratord: config::Concentratord {
                event_url: "ipc:///tmp/relay_concentratord_event".into(),
                command_url: "ipc:///tmp/relay_concentratord_command".into(),
            },
        },
        mappings: config::Mappings {
            channels: vec![868100000, 868300000, 868500000],
            data_rates: vec![config::DataRate {
                modulation: config::Modulation::LORA,
                spreading_factor: 12,
                bandwidth: 125000,
                code_rate: Some(config::CodeRate::Cr45),
                ..Default::default()
            }],
            tx_power: vec![27, 16],
        },
        ..Default::default()
    }
}

async fn init_forwarder(border_gateway: bool) {
    if !border_gateway {
        return;
    }

    let conf = get_config(border_gateway);

    let mut event_sock = zeromq::SubSocket::new();
    event_sock
        .connect(&conf.relay.proxy_api.event_bind)
        .await
        .unwrap();
    event_sock.subscribe("").await.unwrap();

    FORWARDER_EVENT_SOCK
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    let mut cmd_sock = zeromq::ReqSocket::new();
    cmd_sock
        .connect(&conf.relay.proxy_api.command_bind)
        .await
        .unwrap();

    FORWARDER_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    sleep(Duration::from_millis(100)).await;
}

async fn init_backend(border_gateway: bool) {
    let conf = get_config(border_gateway);

    let mut event_sock = zeromq::PubSocket::new();
    cleanup_socket_file(&conf.backend.concentratord.event_url).await;
    event_sock
        .bind(&conf.backend.concentratord.event_url)
        .await
        .unwrap();

    BACKEND_EVENT_SOCK
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    let mut cmd_sock = zeromq::RepSocket::new();
    cleanup_socket_file(&conf.backend.concentratord.command_url).await;
    cmd_sock
        .bind(&conf.backend.concentratord.command_url)
        .await
        .unwrap();

    BACKEND_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    let mut event_sock = zeromq::PubSocket::new();
    cleanup_socket_file(&conf.backend.relay_concentratord.event_url).await;
    event_sock
        .bind(&conf.backend.relay_concentratord.event_url)
        .await
        .unwrap();

    RELAY_BACKEND_EVENT_SOCK
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    let mut cmd_sock = zeromq::RepSocket::new();
    cleanup_socket_file(&conf.backend.relay_concentratord.command_url).await;
    cmd_sock
        .bind(&conf.backend.relay_concentratord.command_url)
        .await
        .unwrap();

    RELAY_BACKEND_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceCell error"))
        .unwrap();

    sleep(Duration::from_millis(300)).await;
}

async fn init_relay() {
    chirpstack_gateway_relay::logging::setup("chirpstack-gateway-relay", log::Level::Trace, false)
        .unwrap();

    tokio::spawn({
        let conf = config::get();

        async move {
            chirpstack_gateway_relay::cmd::root::run(&conf)
                .await
                .unwrap();
        }
    });

    // Respond to Gateway ID requests.
    tokio::spawn(async move {
        let mut cmd_sock = BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let _ = cmd_sock.recv().await;
        cmd_sock
            .send(vec![1, 1, 1, 1, 1, 1, 1, 1].into())
            .await
            .unwrap();
    });

    tokio::spawn(async move {
        let mut cmd_sock = RELAY_BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let _ = cmd_sock.recv().await;
        cmd_sock
            .send(vec![2, 2, 2, 2, 2, 2, 2, 2].into())
            .await
            .unwrap();
    });

    sleep(Duration::from_millis(100)).await;
}

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
