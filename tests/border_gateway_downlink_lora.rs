#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

mod common;

/*
    This tests tests the scenario when the Border Gateway receives a downlink
    LoRaWAN frame which must be transmitted as-is. The Border Gateway acts
    as a normal LoRa gateway in this case.
*/
#[tokio::test]
async fn test_border_gateway_downlink_lora() {
    common::setup(true).await;

    let down = gw::DownlinkFrame {
        downlink_id: 123,
        gateway_id: "0101010101010101".into(),
        items: vec![gw::DownlinkFrameItem {
            phy_payload: vec![9, 8, 7, 6],
            tx_info: Some(gw::DownlinkTxInfo {
                frequency: 868100000,
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
                            seconds: 5,
                            nanos: 0,
                        }),
                    })),
                }),
                context: vec![1, 2, 3, 4],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    // Publish downlink command.
    {
        let mut cmd_sock = common::FORWARDER_COMMAND_SOCK.get().unwrap().lock().await;
        let cmd = gw::Command {
            command: Some(gw::command::Command::SendDownlinkFrame(down.clone())),
        };
        cmd_sock
            .send(
                vec![bytes::Bytes::from(cmd.encode_to_vec())]
                    .try_into()
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    // We expect the same downlink to be received by the concentratord.
    let down_received: gw::DownlinkFrame = {
        let mut cmd_sock = common::BACKEND_COMMAND_SOCK.get().unwrap().lock().await;
        let msg = cmd_sock.recv().await.unwrap();

        let cmd = gw::Command::decode(msg.get(0).cloned().unwrap()).unwrap();
        if let Some(gw::command::Command::SendDownlinkFrame(v)) = cmd.command {
            v
        } else {
            panic!("No DownlinkFrame");
        }
    };

    assert_eq!(down, down_received);
}
