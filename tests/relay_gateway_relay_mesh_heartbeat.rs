use std::time::UNIX_EPOCH;

#[macro_use]
extern crate anyhow;

use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use chirpstack_gateway_mesh::packets;
use tokio::time::{timeout, Duration};
use zeromq::{SocketRecv, SocketSend};

use chirpstack_gateway_mesh::aes128::Aes128Key;

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
            payload_type: packets::PayloadType::Heartbeat,
            hop_count: 1,
        },
        payload: packets::Payload::Heartbeat(packets::HeartbeatPayload {
            relay_id: [1, 2, 3, 4],
            timestamp: UNIX_EPOCH,
            relay_path: vec![],
        }),
        mic: None,
    };
    packet.set_mic(Aes128Key::null()).unwrap();

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

    // We expect packet to be wrapped as 'downlink' and received by the
    // mesh concentratord.
    let down: gw::DownlinkFrame = {
        let mut cmd_sock = common::MESH_BACKEND_COMMAND_SOCK
            .get()
            .unwrap()
            .lock()
            .await;
        let msg = cmd_sock.recv().await.unwrap();

        let cmd = String::from_utf8(msg.get(0).map(|v| v.to_vec()).unwrap()).unwrap();
        assert_eq!("down", cmd);

        gw::DownlinkFrame::decode(msg.get(1).cloned().unwrap()).unwrap()
    };

    let down_item = down.items.first().unwrap();
    let mesh_packet = packets::Packet::from_slice(&down_item.phy_payload).unwrap();

    packet.mhdr.hop_count += 1;
    if let packets::Payload::Heartbeat(v) = &mut packet.payload {
        v.relay_path.push([2, 2, 2, 2]);
    }
    packet.set_mic(Aes128Key::null()).unwrap();

    assert_eq!(packets::Packet::Mesh(packet), mesh_packet);

    // Publish the uplink one more time, this time we expect that it will be discarded.
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
