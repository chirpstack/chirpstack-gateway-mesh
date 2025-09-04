#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use chirpstack_gateway_mesh::packets;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{get_signing_key, Aes128Key};

mod common;

/*
    This tests the scenario when the Relay Gateway receives an uplink LoRaWAN
    frame. The Relay Gateway will then mesh encapsulate this frame, before
    it is forwarded to the Border Gateway.
*/
#[tokio::test]
async fn test_relay_gateway_uplink_lora() {
    common::setup(false).await;

    let up = gw::UplinkFrame {
        phy_payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
        tx_info: Some(gw::UplinkTxInfo {
            frequency: 868300000,
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
            gateway_id: "0101010101010101".to_string(),
            crc_status: gw::CrcStatus::CrcOk.into(),
            rssi: -60,
            snr: 12.0,
            ..Default::default()
        }),
        ..Default::default()
    };

    // Publish uplink event.
    {
        let mut event_sock = common::BACKEND_EVENT_SOCK.get().unwrap().lock().await;
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

    // We expect uplink to be wrapped as 'downlink' and received by the
    // mesh concentratord.
    let down: gw::DownlinkFrame = {
        let mut cmd_sock = common::MESH_BACKEND_COMMAND_SOCK
            .get()
            .unwrap()
            .lock()
            .await;
        let msg = cmd_sock.recv().await.unwrap();

        let cmd = gw::Command::decode(msg.get(0).cloned().unwrap()).unwrap();
        if let Some(gw::command::Command::SendDownlinkFrame(v)) = cmd.command {
            v
        } else {
            panic!("No DownlinkFrame");
        }
    };

    let down_item = down.items.first().unwrap();
    let mesh_packet = packets::MeshPacket::from_slice(&down_item.phy_payload).unwrap();
    let ts = if let packets::Payload::Uplink(pl) = &mesh_packet.payload {
        pl.timestamp
    } else {
        0
    };

    assert_eq!(
        {
            let mut packet = packets::MeshPacket {
                mhdr: packets::MHDR {
                    payload_type: packets::PayloadType::Uplink,
                    hop_count: 1,
                },
                payload: packets::Payload::Uplink(packets::UplinkPayload {
                    metadata: packets::UplinkMetadata {
                        uplink_id: 1,
                        dr: 0,
                        rssi: -60,
                        snr: 12,
                        channel: 1,
                    },
                    timestamp: ts,
                    relay_id: [2, 2, 2, 2],
                    phy_payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
                }),
                mic: None,
            };
            packet.set_mic(get_signing_key(Aes128Key::null())).unwrap();
            packet
        },
        mesh_packet
    );

    assert_eq!(
        &gw::DownlinkTxInfo {
            frequency: 868100000,
            power: 16,
            modulation: Some(gw::Modulation {
                parameters: Some(gw::modulation::Parameters::Lora(gw::LoraModulationInfo {
                    bandwidth: 125000,
                    spreading_factor: 7,
                    code_rate: gw::CodeRate::Cr45.into(),
                    ..Default::default()
                }))
            }),
            timing: Some(gw::Timing {
                parameters: Some(gw::timing::Parameters::Immediately(
                    gw::ImmediatelyTimingInfo {}
                )),
            }),
            ..Default::default()
        },
        down_item.tx_info.as_ref().unwrap()
    );
}
