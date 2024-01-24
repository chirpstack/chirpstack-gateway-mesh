#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use chirpstack_gateway_relay::packets;
use zeromq::{SocketRecv, SocketSend};

mod common;

/*
    This tests the scenario when the Relay Gateway receives an uplink LoRaWAN
    frame. The Relay Gateway will then relay encapsulate this frame, before
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
        event_sock
            .send(
                vec![
                    bytes::Bytes::from("up"),
                    bytes::Bytes::from(up.encode_to_vec()),
                ]
                .try_into()
                .unwrap(),
            )
            .await
            .unwrap();
    }

    // We expect uplink to be wrapped as 'downlink' and received by the
    // relay concentratord.
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
                payload_type: packets::PayloadType::Uplink,
                hop_count: 0
            },
            payload: packets::Payload::Uplink(packets::UplinkPayload {
                metadata: packets::UplinkMetadata {
                    uplink_id: 1,
                    dr: 0,
                    rssi: -60,
                    snr: 12,
                    channel: 1,
                },
                relay_id: [2, 2, 2, 2],
                phy_payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
            }),
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
