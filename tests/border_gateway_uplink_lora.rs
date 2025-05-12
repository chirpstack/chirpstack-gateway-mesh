#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

mod common;

/*
   This tests the scenario when the Border Gateway receives a regular LoRaWAN
   uplink frame. The Border Gateway acts as a normal LoRa gateway in this case.
*/
#[tokio::test]
async fn test_border_gateway_uplink_lora() {
    common::setup(true).await;

    let up = gw::UplinkFrame {
        phy_payload: vec![1, 2, 3, 4, 5, 6, 7, 8],
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
            gateway_id: "0101010101010101".to_string(),
            crc_status: gw::CrcStatus::CrcOk.into(),
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

    // We expect to receive the same uplink.
    let up_received: gw::UplinkFrame = {
        let mut event_sock = common::FORWARDER_EVENT_SOCK.get().unwrap().lock().await;
        let msg = event_sock.recv().await.unwrap();

        let event = gw::Event::decode(msg.get(0).cloned().unwrap()).unwrap();
        if let Some(gw::event::Event::UplinkFrame(v)) = event.event {
            v
        } else {
            panic!("No UplinkFrame");
        }
    };

    // Validate they are equal.
    assert_eq!(up, up_received);
}
