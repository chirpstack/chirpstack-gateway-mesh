use std::time::UNIX_EPOCH;

#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::Aes128Key;
use chirpstack_gateway_mesh::packets;

mod common;

/*
    Thsi tests the scenario that the Border Gateway receives a mesh heartbeat
    packet. The Border Gateway will forward this to the Forwarder application.
*/
#[tokio::test]
async fn test_border_gateway_mesh_heartbeat() {
    common::setup(true).await;

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Heartbeat,
            hop_count: 1,
        },
        payload: packets::Payload::Heartbeat(packets::HeartbeatPayload {
            relay_id: [2, 2, 2, 2],
            timestamp: UNIX_EPOCH,
            relay_path: vec![[1, 2, 3, 4], [5, 6, 7, 8]],
        }),
        mic: None,
    };
    packet.set_mic(Aes128Key::null()).unwrap();

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
        let mut event_sock = common::MESH_BACKEND_EVENT_SOCK.get().unwrap().lock().await;
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

    // We expect to receive the MeshStats to be received by the forwarder.
    let mesh_stats: gw::MeshStats = {
        let mut event_sock = common::FORWARDER_EVENT_SOCK.get().unwrap().lock().await;
        let msg = event_sock.recv().await.unwrap();

        let cmd = String::from_utf8(msg.get(0).map(|v| v.to_vec()).unwrap()).unwrap();
        assert_eq!("mesh_heartbeat", cmd);

        gw::MeshStats::decode(msg.get(1).cloned().unwrap()).unwrap()
    };

    assert_eq!(
        gw::MeshStats {
            gateway_id: "0101010101010101".to_string(),
            time: Some(UNIX_EPOCH.into()),
            relay_id: "02020202".to_string(),
            relay_path: vec!["01020304".to_string(), "05060708".to_string(),],
        },
        mesh_stats
    );
}
