use std::time::UNIX_EPOCH;

#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::Aes128Key;
use chirpstack_gateway_mesh::packets;

mod common;

/*
    Thsi tests the scenario that the Border Gateway receives a mesh heartbeat
    packet. The Border Gateway will forward this to the Forwarder application.
*/
#[tokio::test]
async fn test_border_gateway_mesh_heartbeat() {
    common::setup(true).await;

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Event,
            hop_count: 1,
        },
        payload: packets::Payload::Event(packets::EventPayload {
            relay_id: [2, 2, 2, 2],
            timestamp: UNIX_EPOCH,
            events: vec![packets::Event::Heartbeat(packets::HeartbeatPayload {
                relay_path: vec![
                    packets::RelayPath {
                        relay_id: [1, 2, 3, 4],
                        rssi: -120,
                        snr: -12,
                    },
                    packets::RelayPath {
                        relay_id: [5, 6, 7, 8],
                        rssi: -120,
                        snr: -12,
                    },
                ],
            })],
        }),
        mic: None,
    };
    packet.set_mic(Aes128Key::null()).unwrap();

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
    {
        let mut event_sock = common::MESH_BACKEND_EVENT_SOCK.get().unwrap().lock().await;
        let event = gw::Event {
            event: Some(gw::event::Event::UplinkFrame(up.clone())),
        };
        event_sock
            .send(
                vec![bytes::Bytes::from(event.encode_to_vec())]
                    .try_into()
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    // We expect to receive a Mesh Event.
    let mesh_event: gw::MeshEvent = {
        let mut event_sock = common::FORWARDER_EVENT_SOCK.get().unwrap().lock().await;
        let msg = event_sock.recv().await.unwrap();
        let event = gw::Event::decode(msg.get(0).cloned().unwrap()).unwrap();

        if let Some(gw::event::Event::Mesh(v)) = event.event {
            v
        } else {
            panic!("Event does not contain MeshEvent");
        }
    };

    assert_eq!(
        gw::MeshEvent {
            gateway_id: "0101010101010101".to_string(),
            time: Some(UNIX_EPOCH.into()),
            relay_id: "02020202".to_string(),
            events: vec![gw::MeshEventItem {
                event: Some(gw::mesh_event_item::Event::Heartbeat(
                    gw::MeshEventHeartbeat {
                        relay_path: vec![
                            gw::MeshEventHeartbeatRelayPath {
                                relay_id: "01020304".into(),
                                rssi: -120,
                                snr: -12,
                            },
                            gw::MeshEventHeartbeatRelayPath {
                                relay_id: "05060708".into(),
                                rssi: -120,
                                snr: -12,
                            },
                        ],
                    },
                )),
            },],
        },
        mesh_event
    );
}
