#[macro_use]
extern crate anyhow;

use std::time::Duration;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use tokio::time::{sleep, timeout};
use zeromq::{SocketRecv, SocketSend};

mod common;

/*
    This tests the scenario when the Relay Gateway receives an uplink LoRaWAN
    frame. As the uplink DevAddr does not match the configured filters, it is
    not forwarded.
*/
#[tokio::test]
async fn test_relay_gateway_uplink_lora() {
    common::setup(false).await;

    let up = gw::UplinkFrame {
        phy_payload: vec![0x02 << 5, 1, 2, 3, 4],
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

    // Wait a little bit.
    sleep(Duration::from_millis(200)).await;

    let mut cmd_sock = common::MESH_BACKEND_COMMAND_SOCK
        .get()
        .unwrap()
        .lock()
        .await;

    let res = timeout(Duration::from_millis(100), cmd_sock.recv()).await;
    assert!(res.is_err());
}
