#[macro_use]
extern crate anyhow;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::{get_encryption_key, Aes128Key};
use chirpstack_gateway_mesh::packets;

mod common;

/*
    This tests the scenario when the Border Gateway receives a Mesh Command which
    must be sent to a Relay Gateway.
*/
#[tokio::test]
async fn border_gateway_mesh_command_proprietary() {
    common::setup(true).await;

    let cmd = gw::MeshCommand {
        gateway_id: "0101010101010101".into(),
        relay_id: "02020202".into(),
        commands: vec![gw::MeshCommandItem {
            command: Some(gw::mesh_command_item::Command::Proprietary(
                gw::MeshCommandProprietary {
                    command_type: 200,
                    payload: vec![4, 3, 2, 1],
                },
            )),
        }],
    };

    // Publish command.
    {
        let mut cmd_sock = common::FORWARDER_COMMAND_SOCK.get().unwrap().lock().await;
        let cmd = gw::Command {
            command: Some(gw::command::Command::Mesh(cmd.clone())),
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

    // We expect the wrapped downlink to be received by the mesh concentratord.
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

    // MIC check.
    assert_ne!([0, 0, 0, 0], mesh_packet.mic.unwrap());
    mesh_packet.mic = None;

    // Decrypt.
    mesh_packet
        .decrypt(get_encryption_key(Aes128Key::null()))
        .unwrap();

    if let packets::Payload::Command(v) = &mut mesh_packet.payload {
        // Asser time is ~ now()
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
                payload_type: packets::PayloadType::Command,
                hop_count: 1
            },
            payload: packets::Payload::Command(packets::CommandPayload {
                timestamp: UNIX_EPOCH,
                relay_id: [2, 2, 2, 2],
                commands: vec![packets::Command::Proprietary((200, vec![4, 3, 2, 1])),]
            }),
            mic: None,
        },
        mesh_packet
    );
}
