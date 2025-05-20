use std::sync::OnceLock;
use std::time::Duration;

use tokio::fs::remove_file;
use tokio::sync::Mutex;
use tokio::time::sleep;
use zeromq::{Socket, SocketRecv, SocketSend};

use chirpstack_api::{gw, prost::Message};
use chirpstack_gateway_mesh::config::{self, Configuration};

pub static FORWARDER_EVENT_SOCK: OnceLock<Mutex<zeromq::SubSocket>> = OnceLock::new();
pub static FORWARDER_COMMAND_SOCK: OnceLock<Mutex<zeromq::ReqSocket>> = OnceLock::new();

pub static BACKEND_EVENT_SOCK: OnceLock<Mutex<zeromq::PubSocket>> = OnceLock::new();
pub static BACKEND_COMMAND_SOCK: OnceLock<Mutex<zeromq::RepSocket>> = OnceLock::new();

pub static MESH_BACKEND_EVENT_SOCK: OnceLock<Mutex<zeromq::PubSocket>> = OnceLock::new();
pub static MESH_BACKEND_COMMAND_SOCK: OnceLock<Mutex<zeromq::RepSocket>> = OnceLock::new();

pub async fn setup(border_gateway: bool) {
    let conf = get_config(border_gateway);
    let _ = config::set(conf);
    init_backend(border_gateway).await;
    init_mesh().await;
    init_forwarder(border_gateway).await;
}

pub fn get_config(border_gateway: bool) -> Configuration {
    Configuration {
        mesh: config::Mesh {
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
                event_bind: "ipc:///tmp/gateway_mesh_event".into(),
                command_bind: "ipc:///tmp/gateway_mesh_command".into(),
            },
            max_hop_count: 3,
            ..Default::default()
        },
        backend: config::Backend {
            concentratord: config::Concentratord {
                event_url: "ipc:///tmp/concentratord_event".into(),
                command_url: "ipc:///tmp/concentratord_command".into(),
            },
            mesh_concentratord: config::Concentratord {
                event_url: "ipc:///tmp/mesh_concentratord_event".into(),
                command_url: "ipc:///tmp/mesh_concentratord_command".into(),
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
        events: config::Events {
            heartbeat_interval: Duration::ZERO,
            commands: [
                ("128".into(), vec!["echo".into(), "foo".into()]),
                ("129".into(), vec!["echo".into(), "bar".into()]),
            ]
            .iter()
            .cloned()
            .collect(),
            ..Default::default()
        },
        commands: config::Commands {
            commands: [("130".into(), vec!["wc".into(), "-m".into()])]
                .iter()
                .cloned()
                .collect(),
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
        .connect(&conf.mesh.proxy_api.event_bind)
        .await
        .unwrap();
    event_sock.subscribe("").await.unwrap();

    FORWARDER_EVENT_SOCK
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceLock error"))
        .unwrap();

    let mut cmd_sock = zeromq::ReqSocket::new();
    cmd_sock
        .connect(&conf.mesh.proxy_api.command_bind)
        .await
        .unwrap();

    FORWARDER_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceLock error"))
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
        .map_err(|_| anyhow!("OnceLock error"))
        .unwrap();

    let mut cmd_sock = zeromq::RepSocket::new();
    cleanup_socket_file(&conf.backend.concentratord.command_url).await;
    cmd_sock
        .bind(&conf.backend.concentratord.command_url)
        .await
        .unwrap();

    BACKEND_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceLock error"))
        .unwrap();

    let mut event_sock = zeromq::PubSocket::new();
    cleanup_socket_file(&conf.backend.mesh_concentratord.event_url).await;
    event_sock
        .bind(&conf.backend.mesh_concentratord.event_url)
        .await
        .unwrap();

    MESH_BACKEND_EVENT_SOCK
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceLock error"))
        .unwrap();

    let mut cmd_sock = zeromq::RepSocket::new();
    cleanup_socket_file(&conf.backend.mesh_concentratord.command_url).await;
    cmd_sock
        .bind(&conf.backend.mesh_concentratord.command_url)
        .await
        .unwrap();

    MESH_BACKEND_COMMAND_SOCK
        .set(Mutex::new(cmd_sock))
        .map_err(|_| anyhow!("OnceLock error"))
        .unwrap();

    sleep(Duration::from_millis(300)).await;
}

async fn init_mesh() {
    chirpstack_gateway_mesh::logging::setup("chirpstack-gateway-mesh", log::Level::Trace, false)
        .unwrap();

    tokio::spawn({
        let conf = config::get();

        async move {
            chirpstack_gateway_mesh::cmd::root::run(&conf)
                .await
                .unwrap();
        }
    });

    // Respond to Gateway ID requests.
    tokio::spawn(async move {
        let mut cmd_sock = BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let _ = cmd_sock.recv().await;
        cmd_sock
            .send(
                gw::GetGatewayIdResponse {
                    gateway_id: "0101010101010101".into(),
                }
                .encode_to_vec()
                .into(),
            )
            .await
            .unwrap();
    });

    tokio::spawn(async move {
        let mut cmd_sock = MESH_BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let _ = cmd_sock.recv().await;
        cmd_sock
            .send(
                gw::GetGatewayIdResponse {
                    gateway_id: "0202020202020202".into(),
                }
                .encode_to_vec()
                .into(),
            )
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
