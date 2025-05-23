use std::time::UNIX_EPOCH;

#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use chirpstack_gateway_mesh::packets;
use tokio::time::{timeout, Duration};
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{get_encryption_key, get_signing_key, Aes128Key};

mod common;

/*
    This tests the scenario when the Relay Gateway receives a Mesh Heartbeat packet.
    In this case, the Relay Gateway must relay the payload.
*/
#[tokio::test]
async fn test_relay_gateway_relay_mesh_heartbeat() {
    common::setup(false).await;

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Event,
            hop_count: 1,
        },
        payload: packets::Payload::Event(packets::EventPayload {
            relay_id: [1, 2, 3, 4],
            timestamp: UNIX_EPOCH,
            events: vec![packets::Event::Heartbeat(packets::HeartbeatPayload {
                relay_path: vec![],
            })],
        }),
        mic: None,
    };
    packet
        .encrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();
    packet.set_mic(get_signing_key(Aes128Key::null())).unwrap();

    let up = gw::UplinkFrame {
        phy_payload: packet.to_vec().unwrap(),
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

    // Publish Uplink
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

    // We expect packet to be wrapped as 'downlink' and received by the
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
    let mut mesh_packet = packets::Packet::from_slice(&down_item.phy_payload).unwrap();
    if let packets::Packet::Mesh(pl) = &mut mesh_packet {
        pl.decrypt(get_encryption_key(Aes128Key::null())).unwrap();
    }

    packet
        .decrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();
    packet.mhdr.hop_count += 1;
    if let packets::Payload::Event(v) = &mut packet.payload {
        for event in &mut v.events {
            if let packets::Event::Heartbeat(v) = event {
                v.relay_path.push(packets::RelayPath {
                    relay_id: [2, 2, 2, 2],
                    rssi: -60,
                    snr: 12,
                });
            }
        }
    }
    packet
        .encrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();
    packet.set_mic(get_signing_key(Aes128Key::null())).unwrap();
    packet
        .decrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();
    assert_eq!(packets::Packet::Mesh(packet), mesh_packet);

    // Publish the uplink one more time, this time we expect that it will be discarded.
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

    // As the item has beem discarded, receiving from the cmd socket should timeout.
    {
        let mut cmd_sock = common::MESH_BACKEND_COMMAND_SOCK
            .get()
            .unwrap()
            .lock()
            .await;

        let resp = timeout(Duration::from_secs(1), cmd_sock.recv()).await;
        assert!(resp.is_err());
    }
}
