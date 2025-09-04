#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{get_signing_key, Aes128Key};
use chirpstack_gateway_mesh::packets;

mod common;

/*
    This tests the scenario when the Border Gateway receives a mesh encapsulated
    LoRaWAN uplink frame. The Border Gateway will unwrap the LoRaWAN frame before
    forwarding it to the Forwarder application.
*/
#[tokio::test]
async fn test_border_gateway_uplink_mesh() {
    common::setup(true).await;

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Uplink,
            hop_count: 1,
        },
        payload: packets::Payload::Uplink(packets::UplinkPayload {
            metadata: packets::UplinkMetadata {
                uplink_id: 123,
                dr: 0,
                rssi: -60,
                snr: 6,
                channel: 2,
            },
            timestamp: 0,
            relay_id: [1, 2, 3, 4],
            phy_payload: vec![9, 8, 7, 6],
        }),
        mic: None,
    };
    packet.set_mic(get_signing_key(Aes128Key::null())).unwrap();

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
            uplink_id: 123456,
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

    // We expect to receive the unwrapped uplink to be received by the forwarder.
    let up: gw::UplinkFrame = {
        let mut event_sock = common::FORWARDER_EVENT_SOCK.get().unwrap().lock().await;
        let msg = event_sock.recv().await.unwrap();

        let event = gw::Event::decode(msg.get(0).cloned().unwrap()).unwrap();
        if let Some(gw::event::Event::UplinkFrame(v)) = event.event {
            v
        } else {
            panic!("No UplinkFrame");
        }
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
    assert_eq!("0101010101010101", rx_info.gateway_id);
    assert_eq!(-60, rx_info.rssi);
    assert_eq!(6.0, rx_info.snr);
    assert_eq!(vec![1, 2, 3, 1, 2, 3, 4, 0, 123], rx_info.context);
    assert_eq!(gw::CrcStatus::CrcOk as i32, rx_info.crc_status);
    assert_eq!("01020304", rx_info.metadata.get("relay_id").unwrap());
    assert_eq!("1", rx_info.metadata.get("hop_count").unwrap());
    assert_eq!("123", rx_info.metadata.get("uplink_id").unwrap());
    assert_eq!(123, rx_info.uplink_id);
    assert!(rx_info.metadata.contains_key("mesh_delay_ms"));
}
