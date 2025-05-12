use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use chirpstack_gateway_mesh::packets;
use zeromq::SocketRecv;

use chirpstack_gateway_mesh::heartbeat;

mod common;

/*
    This tests the scenario when the Relay Gateway sends its periodic heartbeat.
*/
#[tokio::test]
async fn test_relay_gateway_mesh_heartbeat() {
    common::setup(false).await;
    let _ = heartbeat::report_heartbeat().await;

    // We expect the heartbeat to be received by the mesh concentratord as
    // a downlink frame.
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
    let mut mesh_packet = packets::MeshPacket::from_slice(&down_item.phy_payload).unwrap();
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
                events: vec![packets::Event::Heartbeat(packets::HeartbeatPayload {
                    relay_path: vec![]
                }),],
            }),
            mic: None,
        },
        mesh_packet
    );
}
