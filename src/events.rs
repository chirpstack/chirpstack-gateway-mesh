use std::collections::HashMap;
use std::time::SystemTime;

use anyhow::Result;
use chirpstack_api::gw;
use log::{error, info};
use rand::random;
use tokio::process::Command;
use tokio::sync::OnceCell;
use tokio::time::sleep;

use crate::aes128::{get_encryption_key, get_signing_key, Aes128Key};
use crate::backend;
use crate::config::{self, Configuration};
use crate::helpers;
use crate::mesh::get_mesh_frequency;
use crate::packets;

static COMMANDS: OnceCell<HashMap<u8, Vec<String>>> = OnceCell::const_new();

pub async fn setup(conf: &Configuration) -> Result<()> {
    // Only Relay Gateways report events.
    if conf.mesh.border_gateway {
        return Ok(());
    }

    // Set commands.
    COMMANDS
        .set(
            conf.events
                .commands
                .iter()
                .map(|(k, v)| (k.parse().unwrap(), v.clone()))
                .collect(),
        )
        .map_err(|_| anyhow!("OnceCell set error"))?;

    // Setup heartbeat event loop.
    if !conf.events.heartbeat_interval.is_zero() {
        info!(
            "Starting heartbeat loop, heartbeat_interval: {:?}",
            conf.events.heartbeat_interval
        );

        tokio::spawn({
            let heartbeat_interval = conf.events.heartbeat_interval;

            async move {
                loop {
                    if let Err(e) = report_heartbeat().await {
                        error!("Report heartbeat error, error: {}", e);
                    }
                    sleep(heartbeat_interval).await;
                }
            }
        });
    }

    // Setup event-set loops.
    for event_set in &conf.events.sets {
        info!(
            "Starting event-set loop, events: {:?}, interval: {:?}",
            event_set.events, event_set.interval
        );

        tokio::spawn({
            let events = event_set.events.clone();
            let interval = event_set.interval;

            async move {
                loop {
                    sleep(interval).await;
                    if let Err(e) = report_events(&events).await {
                        error!("Report event-set error, error: {}", e);
                    }
                }
            }
        });
    }

    Ok(())
}

pub async fn report_heartbeat() -> Result<()> {
    info!("Sending heartbeat event");
    send_events(vec![packets::Event::Heartbeat(packets::HeartbeatPayload {
        relay_path: vec![],
    })])
    .await
}

pub async fn report_events(typs: &[u8]) -> Result<()> {
    let mut events = Vec::new();
    for typ in typs {
        events.push(get_event(*typ).await?);
    }
    info!("Sending events, events: {:?}", typs);
    send_events(events).await
}

async fn get_event(typ: u8) -> Result<packets::Event> {
    let args = COMMANDS
        .get()
        .ok_or_else(|| anyhow!("COMMANDS is not set"))?
        .get(&typ)
        .ok_or_else(|| anyhow!("Event type {} is not configured", typ))?;

    if args.is_empty() {
        return Err(anyhow!("Command for event type {} is empty", typ));
    }

    let mut cmd = Command::new(&args[0]);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    let output = cmd.output().await?;

    Ok(packets::Event::Proprietary((typ, output.stdout)))
}

pub async fn send_events(events: Vec<packets::Event>) -> Result<()> {
    let conf = config::get();

    let mut packet = packets::MeshPacket {
        mhdr: packets::MHDR {
            payload_type: packets::PayloadType::Event,
            hop_count: 1,
        },
        payload: packets::Payload::Event(packets::EventPayload {
            timestamp: SystemTime::now(),
            relay_id: backend::get_relay_id().await?,
            events,
        }),
        mic: None,
    };
    packet.encrypt(get_encryption_key(Aes128Key::null()))?;
    packet.set_mic(get_signing_key(conf.mesh.signing_key))?;

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
        "Sending event packet, downlink_id: {}, mesh_packet: {}",
        pl.downlink_id, packet
    );

    backend::mesh(pl).await
}
