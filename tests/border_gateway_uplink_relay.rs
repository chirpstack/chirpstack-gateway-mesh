#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_relay::packets;

mod common;

/*
    This tests the scenario when the Border Gateway receives a relay encapsulated
    LoRaWAN uplink frame. The Border Gateway will unwrap the LoRaWAN frame before
    forwarding it to the Forwarder application.
*/
#[tokio::test]
async fn test_border_gateway_uplink_relay() {
    common::setup(true).await;

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
    {
        let mut event_sock = common::RELAY_BACKEND_EVENT_SOCK.get().unwrap().lock().await;
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

    // We expect to receive the unwrapped uplink to be received by the forwarder.
    let up: gw::UplinkFrame = {
        let mut event_sock = common::FORWARDER_EVENT_SOCK.get().unwrap().lock().await;
        let msg = event_sock.recv().await.unwrap();

        let cmd = String::from_utf8(msg.get(0).map(|v| v.to_vec()).unwrap()).unwrap();
        assert_eq!("up", cmd);

        gw::UplinkFrame::decode(msg.get(1).cloned().unwrap()).unwrap()
    };

    // Validate PHYPayload
    assert_eq!(vec![9, 8, 7, 6], up.phy_payload);

    // Validate TxInfo
    let tx_info = up.tx_info.as_ref().unwrap();
    assert_eq!(
        &gw::UplinkTxInfo {
            frequency: 868500000,
            modulation: Some(gw::Modulation {
                parameters: Some(gw::modulation::Parameters::Lora(gw::LoraModulationInfo {
                    bandwidth: 125000,
                    spreading_factor: 12,
                    code_rate: gw::CodeRate::Cr45.into(),
                    ..Default::default()
                })),
            }),
        },
        tx_info
    );

    // Validate RxInfo (GatewayID, context, RSSI & SNR)
    let rx_info = up.rx_info.as_ref().unwrap();
    assert_eq!(
        &gw::UplinkRxInfo {
            gateway_id: "0101010101010101".to_string(),
            rssi: -60,
            snr: 6.0,
            context: vec![1, 2, 3, 1, 2, 3, 4, 0, 123],
            crc_status: gw::CrcStatus::CrcOk.into(),
            metadata: [
                ("relay_id".to_string(), "01020304".to_string()),
                ("hop_count".to_string(), "1".to_string()),
            ]
            .iter()
            .cloned()
            .collect(),
            ..Default::default()
        },
        rx_info
    );
}
