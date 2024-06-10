use std::time::SystemTime;

use anyhow::Result;
use chirpstack_api::gw;
use log::{error, info};
use rand::random;
use tokio::time::sleep;

use crate::backend;
use crate::config::{self, Configuration};
use crate::helpers;
use crate::mesh::get_mesh_frequency;
use crate::packets;

pub async fn setup(conf: &Configuration) -> Result<()> {
    // Only Relay gatewways need to report stats as the Border Gateway is already internet
    // connected and reports status through the Concentratord.
    if conf.mesh.border_gateway || conf.mesh.stats_interval.is_zero() {
        return Ok(());
    }

    info!(
        "Starting stats loop, stats_interval: {:?}",
        conf.mesh.stats_interval
    );

    tokio::spawn({
        let stats_interval = conf.mesh.stats_interval;

        async move {
            loop {
                if let Err(e) = report_stats().await {
                    error!("Report stats error, error: {}", e);
                }
                sleep(stats_interval).await;
            }
        }
    });

    Ok(())
}

pub async fn report_stats() -> Result<()> {
    let conf = config::get();

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Stats,
            hop_count: 1,
        },
        payload: packets::Payload::Stats(packets::StatsPayload {
            timestamp: SystemTime::now(),
            relay_id: backend::get_relay_id().await.unwrap_or_default(),
            relay_path: vec![],
        }),
        mic: None,
    };
    packet.set_mic(conf.mesh.signing_key)?;

    let pl = gw::DownlinkFrame {
        downlink_id: random(),
        items: vec![gw::DownlinkFrameItem {
            phy_payload: packet.to_vec()?,
            tx_info: Some(gw::DownlinkTxInfo {
                frequency: get_mesh_frequency(&conf)?,
                modulation: Some(helpers::data_rate_to_gw_modulation(
                    &conf.mesh.data_rate,
                    false,
                )),
                power: conf.mesh.tx_power,
                timing: Some(gw::Timing {
                    parameters: Some(gw::timing::Parameters::Immediately(
                        gw::ImmediatelyTimingInfo {},
                    )),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    info!(
        "Sending stats packet, downlink_id: {}, mesh_packet: {}",
        pl.downlink_id, packet
    );
    backend::mesh(&pl).await
}
