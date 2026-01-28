#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::{prost::Message, prost_types};
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{Aes128Key, get_signing_key};
use chirpstack_gateway_mesh::{mesh, packets};

mod common;

/*
    This tests the scenario when the Relay Gateway receives a mesh encapsulated
    downlink LoRaWAN frame. The Relay Gateway will unwrap the LoRaWAN frame before
    sending the downlink to the device.
*/
#[tokio::test]
async fn test_relay_gateway_downlink_lora() {
    common::setup(false).await;

    let uplink_id = mesh::store_uplink_context(&[5, 4, 3, 2, 1]);

    let mut down_packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Downlink,
            hop_count: 1,
        },
        payload: packets::Payload::Downlink(packets::DownlinkPayload {
            metadata: packets::DownlinkMetadata {
                uplink_id,
                dr: 0,
                frequency: 867100000,
                tx_power: 1,
                delay: 5,
            },
            relay_id: [2, 2, 2, 2],
            phy_payload: vec![9, 8, 7, 6, 5],
        }),
        mic: None,
    };
    down_packet
        .set_mic(get_signing_key(Aes128Key::null()))
        .unwrap();

    // The packet that we received from the Border Gateway that must be relayed to
    // the End Device.
    let up = gw::UplinkFrame {
        phy_payload: down_packet.to_vec().unwrap(),
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
            gateway_id: "0101010101010101".into(),
            crc_status: gw::CrcStatus::CrcOk.into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Publish uplink.
    // (we simulate that we receive the Mesh encapsulated downlink)
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

    // We expect that the unwrapped downlink was sent to the concentratord.
    let mut down: gw::DownlinkFrame = {
        let mut cmd_sock = common::BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let msg = cmd_sock.recv().await.unwrap();

        let cmd = gw::Command::decode(msg.get(0).cloned().unwrap()).unwrap();
        if let Some(gw::command::Command::SendDownlinkFrame(v)) = cmd.command {
            v
        } else {
            panic!("No DownlinkFrame");
        }
    };

    assert_ne!(0, down.downlink_id);
    down.downlink_id = 0;

    assert_eq!(
        gw::DownlinkFrame {
            gateway_id: "0101010101010101".into(),
            items: vec![gw::DownlinkFrameItem {
                phy_payload: vec![9, 8, 7, 6, 5],
                tx_info: Some(gw::DownlinkTxInfo {
                    frequency: 867100000,
                    power: 16,
                    modulation: Some(gw::Modulation {
                        parameters: Some(gw::modulation::Parameters::Lora(
                            gw::LoraModulationInfo {
                                bandwidth: 125000,
                                spreading_factor: 12,
                                code_rate: gw::CodeRate::Cr45.into(),
                                polarization_inversion: true,
                                ..Default::default()
                            }
                        ))
                    }),
                    timing: Some(gw::Timing {
                        parameters: Some(gw::timing::Parameters::Delay(gw::DelayTimingInfo {
                            delay: Some(prost_types::Duration {
                                seconds: 5,
                                nanos: 0
                            }),
                        })),
                    }),
                    context: vec![5, 4, 3, 2, 1],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        },
        down
    );
}
