#[macro_use]
extern crate anyhow;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{get_encryption_key, get_signing_key, Aes128Key};
use chirpstack_gateway_mesh::packets;

mod common;

/*
    This test the scenario when the Relay Gateway receives a Mesh Command. THe
    Relay Gateway will execute this command and then sends back the response as
    a Mesh Event.
*/
#[tokio::test]
async fn test_relay_gateway_mesh_command_proprietary() {
    common::setup(false).await;

    let mut cmd_packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Command,
            hop_count: 1,
        },
        payload: packets::Payload::Command(packets::CommandPayload {
            timestamp: SystemTime::now(),
            relay_id: [2, 2, 2, 2],
            commands: vec![packets::Command::Proprietary((
                130,
                "hello".as_bytes().to_vec(),
            ))],
        }),
        mic: None,
    };
    cmd_packet
        .encrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();
    cmd_packet
        .set_mic(get_signing_key(Aes128Key::null()))
        .unwrap();

    // The packet that we received from the Border Gateway.
    let up = gw::UplinkFrame {
        phy_payload: cmd_packet.to_vec().unwrap(),
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
    // (we simulate that we receive the Mesh command)
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

    // We expect that the Relay Gateway responds to the Mesh Command by sending
    // a Mesh Event back.
    let mut down: gw::DownlinkFrame = {
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

    assert_ne!(0, down.downlink_id);
    down.downlink_id = 0;

    let down_item = down.items.first().unwrap();
    let mut mesh_packet = packets::MeshPacket::from_slice(&down_item.phy_payload).unwrap();

    // Decrypt.
    mesh_packet
        .decrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();

    // MIC.
    assert_ne!([0, 0, 0, 0], mesh_packet.mic.unwrap());
    mesh_packet.mic = None;

    if let packets::Payload::Event(v) = &mut mesh_packet.payload {
        // Assert the time is ~ now()
        assert!(
            SystemTime::now()
                .duration_since(v.timestamp)
                .unwrap_or_default()
                < Duration::from_secs(5)
        );
        v.timestamp = UNIX_EPOCH;
    }

    assert_eq!(
        packets::MeshPacket {
            mhdr: packets::MHDR {
                payload_type: packets::PayloadType::Event,
                hop_count: 1,
            },
            payload: packets::Payload::Event(packets::EventPayload {
                relay_id: [2, 2, 2, 2],
                timestamp: UNIX_EPOCH,
                events: vec![packets::Event::Proprietary((130, vec![53, 10])),], // 53 = 5 in ascii
            }),
            mic: None,
        },
        mesh_packet
    );
}
