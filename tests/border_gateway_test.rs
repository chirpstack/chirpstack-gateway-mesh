#[macro_use]
extern crate anyhow;

use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::{Lazy, OnceCell};
use tokio::task;
use tokio::time::sleep;

use chirpstack_api::gw::{self, modulation};
use chirpstack_api::prost::Message;
use chirpstack_gateway_relay::config::{self, Configuration};
use chirpstack_gateway_relay::packets;

static TEST_SYNC: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static RELAY_INIT: OnceCell<bool> = OnceCell::new();

static FORWARDER_EVENT_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();
static FORWARDER_COMMAND_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();

static BACKEND_EVENT_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();
static BACKEND_COMMAND_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();

static RELAY_BACKEND_EVENT_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();
static RELAY_BACKEND_COMMAND_SOCK: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();

fn get_config() -> Configuration {
    Configuration {
        relay: config::Relay {
            frequencies: vec![868100000],
            data_rate: config::DataRate {
                modulation: config::Modulation::LORA,
                spreading_factor: 7,
                bandwidth: 125000,
                code_rate: Some(config::CodeRate::Cr45),
                ..Default::default()
            },
            tx_power: 16,
            border_gateway: true,
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
        channels: vec![868100000, 868300000, 868500000],
        data_rates: vec![config::DataRate {
            modulation: config::Modulation::LORA,
            spreading_factor: 12,
            bandwidth: 125000,
            code_rate: Some(config::CodeRate::Cr45),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn init_forwarder() {
    let conf = get_config();
    let zmq_ctx = zmq::Context::new();

    if FORWARDER_EVENT_SOCK.get().is_none() {
        let event_sock = zmq_ctx.socket(zmq::SUB).unwrap();
        event_sock
            .connect(&conf.relay.proxy_api.event_bind)
            .unwrap();
        event_sock.set_subscribe("".as_bytes()).unwrap();
        FORWARDER_EVENT_SOCK
            .set(Mutex::new(event_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }

    if FORWARDER_COMMAND_SOCK.get().is_none() {
        let cmd_sock = zmq_ctx.socket(zmq::REQ).unwrap();
        cmd_sock
            .connect(&conf.relay.proxy_api.command_bind)
            .unwrap();
        FORWARDER_COMMAND_SOCK
            .set(Mutex::new(cmd_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }
}

fn init_backend() {
    let conf = get_config();
    let zmq_ctx = zmq::Context::new();

    if BACKEND_EVENT_SOCK.get().is_none() {
        let event_sock = zmq_ctx.socket(zmq::PUB).unwrap();
        event_sock
            .bind(&conf.backend.concentratord.event_url)
            .unwrap();
        BACKEND_EVENT_SOCK
            .set(Mutex::new(event_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }

    if BACKEND_COMMAND_SOCK.get().is_none() {
        let cmd_sock = zmq_ctx.socket(zmq::REP).unwrap();
        cmd_sock
            .bind(&conf.backend.concentratord.command_url)
            .unwrap();
        BACKEND_COMMAND_SOCK
            .set(Mutex::new(cmd_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }

    if RELAY_BACKEND_EVENT_SOCK.get().is_none() {
        let event_sock = zmq_ctx.socket(zmq::PUB).unwrap();
        event_sock
            .bind(&conf.backend.relay_concentratord.event_url)
            .unwrap();
        RELAY_BACKEND_EVENT_SOCK
            .set(Mutex::new(event_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }

    if RELAY_BACKEND_COMMAND_SOCK.get().is_none() {
        let cmd_sock = zmq_ctx.socket(zmq::REP).unwrap();
        cmd_sock
            .bind(&conf.backend.relay_concentratord.command_url)
            .unwrap();
        RELAY_BACKEND_COMMAND_SOCK
            .set(Mutex::new(cmd_sock))
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();
    }
}

async fn init_relay() {
    if RELAY_INIT.get().is_none() {
        chirpstack_gateway_relay::logging::setup(
            "chirpstack-gateway-relay",
            log::Level::Debug,
            false,
        )
        .unwrap();

        tokio::spawn({
            let conf = config::get();

            async move {
                chirpstack_gateway_relay::cmd::root::run(&conf)
                    .await
                    .unwrap();
            }
        });

        RELAY_INIT
            .set(true)
            .map_err(|_| anyhow!("OnceCell error"))
            .unwrap();

        tokio::task::spawn_blocking({
            move || {
                let cmd_sock = BACKEND_COMMAND_SOCK.get().unwrap().lock().unwrap();
                let _ = cmd_sock.recv_multipart(0).unwrap();
                let gw_id: Vec<u8> = vec![1, 1, 1, 1, 1, 1, 1, 1];
                cmd_sock.send(&gw_id, 0).unwrap();
            }
        })
        .await
        .unwrap();

        tokio::task::spawn_blocking({
            move || {
                let cmd_sock = RELAY_BACKEND_COMMAND_SOCK.get().unwrap().lock().unwrap();
                let _ = cmd_sock.recv_multipart(0).unwrap();
                let gw_id: Vec<u8> = vec![2, 2, 2, 2, 2, 2, 2, 2];
                cmd_sock.send(&gw_id, 0).unwrap();
            }
        })
        .await
        .unwrap();

        sleep(Duration::from_millis(100)).await;
    }
}

// The Relay Gateway receives a relayed uplink.
// We expect that the "unwrapped" uplink is proxied to the forwarder using the
// relay context.
#[tokio::test]
async fn test_uplink_relay_frame() {
    let _guard = TEST_SYNC.lock().unwrap();
    let _ = config::set(get_config());
    init_backend();
    init_relay().await;
    init_forwarder();

    let packet = packets::RelayPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Uplink,
            hop_count: 0,
        },
        payload: packets::Payload::Uplink(packets::UplinkPayload {
            metadata: packets::UplinkMetadata {
                uplink_id: 123,
                dr: 0,
                rssi: -60,
                snr: 6,
                channel: 2,
            },
            relay_id: [1, 2, 3, 4],
            phy_payload: vec![9, 8, 7, 6],
        }),
    };

    let up = gw::UplinkFrame {
        phy_payload: packet.to_vec().unwrap(),
        tx_info: Some(gw::UplinkTxInfo {
            frequency: 868100000,
            modulation: Some(gw::Modulation {
                parameters: Some(gw::modulation::Parameters::Lora(gw::LoraModulationInfo {
                    bandwidth: 125000,
                    spreading_factor: 12,
                    code_rate: gw::CodeRate::Cr45.into(),
                    ..Default::default()
                })),
            }),
        }),
        rx_info: Some(gw::UplinkRxInfo {
            crc_status: gw::CrcStatus::CrcOk.into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Publish uplink event.
    task::spawn_blocking({
        let up = up.clone();

        move || {
            let event_sock = RELAY_BACKEND_EVENT_SOCK.get().unwrap().lock().unwrap();
            event_sock.send("up", zmq::SNDMORE).unwrap();
            event_sock.send(up.encode_to_vec(), 0).unwrap();
        }
    })
    .await
    .unwrap();

    // We expect to receive the unwrapped uplink to be received by the forwarder.
    let up: gw::UplinkFrame = task::spawn_blocking({
        move || -> gw::UplinkFrame {
            let event_sock = FORWARDER_EVENT_SOCK.get().unwrap().lock().unwrap();
            let msg = event_sock.recv_multipart(0).unwrap();
            let cmd = String::from_utf8(msg[0].clone()).unwrap();
            assert_eq!("up", cmd);
            gw::UplinkFrame::decode(&*msg[1]).unwrap()
        }
    })
    .await
    .unwrap();

    // Validate PHYPayload
    assert_eq!(vec![9, 8, 7, 6], up.phy_payload);

    // Validate TxInfo

    // Validate RxInfo (RSSI & SNR)
}

// #[tokio::test]
// async fn test_uplink_lora_frame() {
//     let _guard = TEST_SYNC.lock().unwrap();
//     let _ = config::set(get_config());
//     init_forwarder();
//     init_backend();
// }

// #[tokio::test]
// async fn test_downlink_relay_frame() {
//     let _guard = TEST_SYNC.lock().unwrap();
//     let _ = config::set(get_config());
//     init_forwarder();
//     init_backend();
// }

// #[tokio::test]
// async fn test_downlink_lora_frame() {
//     let _guard = TEST_SYNC.lock().unwrap();
//     let _ = config::set(get_config());
//     init_forwarder();
//     init_backend();
// }
