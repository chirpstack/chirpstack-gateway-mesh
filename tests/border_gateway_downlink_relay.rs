#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_relay::packets;

mod common;

/*
    This tests the scenario when the Border Gateway receives a downlink which must
    be relay encapsulated and forwarded to the Relay Gateway.
*/
#[tokio::test]
async fn test_border_gateway_downlink_relay() {
    common::setup(true).await;

    let down = gw::DownlinkFrame {
        downlink_id: 1,
        gateway_id: "0101010101010101".into(),
        items: vec![gw::DownlinkFrameItem {
            phy_payload: vec![9, 8, 7, 6],
            tx_info: Some(gw::DownlinkTxInfo {
                frequency: 868500000,
                power: 16,
                modulation: Some(gw::Modulation {
                    parameters: Some(gw::modulation::Parameters::Lora(gw::LoraModulationInfo {
                        bandwidth: 125000,
                        spreading_factor: 12,
                        code_rate: gw::CodeRate::Cr45.into(),
                        polarization_inversion: true,
                        ..Default::default()
                    })),
                }),
                timing: Some(gw::Timing {
                    parameters: Some(gw::timing::Parameters::Delay(gw::DelayTimingInfo {
                        delay: Some(prost_types::Duration {
                            seconds: 3,
                            ..Default::default()
                        }),
                    })),
                }),
                context: vec![1, 2, 3, 1, 2, 3, 4, 0, 123],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    // Publish downlink command.
    {
        let mut cmd_sock = common::FORWARDER_COMMAND_SOCK.get().unwrap().lock().await;
        cmd_sock
            .send(
                vec![
                    bytes::Bytes::from("down"),
                    bytes::Bytes::from(down.encode_to_vec()),
                ]
                .try_into()
                .unwrap(),
            )
            .await
            .unwrap();
    }

    // We expect the wrapped downlink to be received by the relay concentratord.
    let down: gw::DownlinkFrame = {
        let mut cmd_sock = common::RELAY_BACKEND_COMMAND_SOCK
            .get()
            .unwrap()
            .lock()
            .await;
        let msg = cmd_sock.recv().await.unwrap();

        let cmd = String::from_utf8(msg.get(0).map(|v| v.to_vec()).unwrap()).unwrap();
        assert_eq!("down", cmd);

        gw::DownlinkFrame::decode(msg.get(1).cloned().unwrap()).unwrap()
    };

    let down_item = down.items.get(0).unwrap();
    let relay_packet = packets::RelayPacket::from_slice(&down_item.phy_payload).unwrap();

    assert_eq!(
        packets::RelayPacket {
            mhdr: packets::MHDR {
                payload_type: packets::PayloadType::Downlink,
                hop_count: 1,
            },
            payload: packets::Payload::Downlink(packets::DownlinkPayload {
                metadata: packets::DownlinkMetadata {
                    uplink_id: 123,
                    dr: 0,
                    frequency: 868500000,
                    tx_power: 1,
                    delay: 3,
                },
                relay_id: [1, 2, 3, 4],
                phy_payload: vec![9, 8, 7, 6],
            })
        },
        relay_packet
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
