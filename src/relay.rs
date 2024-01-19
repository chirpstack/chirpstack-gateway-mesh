use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use chirpstack_api::gw;
use log::warn;
use once_cell::sync::Lazy;
use rand::random;

use crate::{
    backend,
    config::{self, Configuration},
    helpers,
    packets::{
        self, DownlinkMetadata, Payload, PayloadType, RelayPacket, UplinkMetadata, UplinkPayload,
        MHDR,
    },
    proxy,
};

static CTX_PREFIX: [u8; 3] = [1, 2, 3];
static RELAY_CHANNEL: Mutex<usize> = Mutex::new(0);
static UPLINK_ID: Mutex<u16> = Mutex::new(0);
static UPLINK_CONTEXT: Lazy<Mutex<HashMap<u16, Vec<u8>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

// Handle LoRaWAN payload (non-proprietary).
pub async fn handle_uplink(border_gateway: bool, pl: gw::UplinkFrame) -> Result<()> {
    match border_gateway {
        true => proxy_uplink_lora_packet(&pl).await,
        false => relay_uplink_lora_packet(&pl).await,
    }
}

// Handle Proprietary LoRaWAN payload (relay encapsulated).
pub async fn handle_relay(border_gateway: bool, pl: gw::UplinkFrame) -> Result<()> {
    let packet = RelayPacket::from_slice(&pl.phy_payload)?;

    match border_gateway {
        // In this case we only care about proxy-ing relayed uplinks
        true => match packet.mhdr.payload_type {
            PayloadType::Uplink => proxy_uplink_relay_packet(&pl, packet).await,
            _ => Ok(()),
        },
        false => relay_relay_packet(&pl, packet).await,
    }
}

pub async fn handle_downlink(pl: gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    if let Some(first_item) = pl.items.first() {
        let tx_info = first_item
            .tx_info
            .as_ref()
            .ok_or_else(|| anyhow!("tx_info is None"))?;

        // Check if context has the CTX_PREFIX, if not we just proxy the downlink payload.
        if tx_info.context.len() < CTX_PREFIX.len()
            || !tx_info.context[0..CTX_PREFIX.len()].eq(&CTX_PREFIX)
        {
            return proxy_downlink_lora_packet(&pl).await;
        }
    }

    relay_downlink_lora_packet(&pl).await
}

async fn proxy_downlink_lora_packet(pl: &gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    backend::send_downlink(pl).await
}

async fn proxy_uplink_lora_packet(pl: &gw::UplinkFrame) -> Result<()> {
    proxy::send_uplink(pl).await
}

async fn proxy_uplink_relay_packet(pl: &gw::UplinkFrame, packet: RelayPacket) -> Result<()> {
    let relay_pl = match &packet.payload {
        Payload::Uplink(v) => v,
        _ => {
            return Err(anyhow!("Expected Uplink payload"));
        }
    };

    let mut pl = pl.clone();

    if let Some(rx_info) = &mut pl.rx_info {
        // Set metadata.
        rx_info.metadata.insert(
            "hop_count".to_string(),
            (packet.mhdr.hop_count + 1).to_string(),
        );
        rx_info
            .metadata
            .insert("relay_id".to_string(), hex::encode(relay_pl.relay_id));

        // Set RSSI and SNR.
        rx_info.snr = relay_pl.metadata.snr.into();
        rx_info.rssi = relay_pl.metadata.rssi.into();

        // Set context.
        rx_info.context = {
            let mut ctx = Vec::with_capacity(CTX_PREFIX.len() + 6); // Relay ID = 4 + Uplink ID = 2
            ctx.extend_from_slice(&CTX_PREFIX);
            ctx.extend_from_slice(&relay_pl.relay_id);
            ctx.extend_from_slice(&relay_pl.metadata.uplink_id.to_be_bytes());
            ctx
        };
    }

    // Set TxInfo.
    if let Some(tx_info) = &mut pl.tx_info {
        tx_info.frequency = helpers::chan_to_frequency(relay_pl.metadata.channel)?;
        tx_info.modulation = Some(helpers::dr_to_modulation(relay_pl.metadata.dr, false)?);
    }

    // Set original PHYPayload.
    pl.phy_payload = relay_pl.phy_payload.clone();

    proxy::send_uplink(&pl).await
}

async fn relay_relay_packet(_: &gw::UplinkFrame, mut packet: RelayPacket) -> Result<()> {
    let conf = config::get();

    // Increment hop count.
    packet.mhdr.hop_count += 1;

    if packet.mhdr.hop_count > conf.relay.max_hop_count {
        return Err(anyhow!("Max hop count exceeded"));
    }

    let pl = gw::DownlinkFrame {
        downlink_id: random(),
        items: vec![gw::DownlinkFrameItem {
            phy_payload: packet.to_vec()?,
            tx_info: Some(gw::DownlinkTxInfo {
                frequency: get_relay_frequency(&conf)?,
                modulation: Some(helpers::data_rate_to_gw_modulation(
                    &conf.relay.data_rate,
                    false,
                )),
                power: conf.relay.tx_power,
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

    backend::relay(&pl).await
}

async fn relay_uplink_lora_packet(pl: &gw::UplinkFrame) -> Result<()> {
    let conf = config::get();

    let rx_info = pl
        .rx_info
        .as_ref()
        .ok_or_else(|| anyhow!("rx_info is None"))?;
    let tx_info = pl
        .tx_info
        .as_ref()
        .ok_or_else(|| anyhow!("tx_info is None"))?;
    let modulation = tx_info
        .modulation
        .as_ref()
        .ok_or_else(|| anyhow!("modulation is None"))?;

    let packet = RelayPacket {
        mhdr: MHDR {
            payload_type: PayloadType::Uplink,
            hop_count: 0,
        },
        payload: Payload::Uplink(UplinkPayload {
            metadata: UplinkMetadata {
                uplink_id: store_uplink_context(&rx_info.context),
                dr: helpers::modulation_to_dr(modulation)?,
                channel: helpers::frequency_to_chan(tx_info.frequency)?,
                rssi: rx_info.rssi as i16,
                snr: rx_info.snr as i8,
            },
            relay_id: backend::get_relay_id()?,
            phy_payload: pl.phy_payload.clone(),
        }),
    };

    let pl = gw::DownlinkFrame {
        downlink_id: random(),
        items: vec![gw::DownlinkFrameItem {
            phy_payload: packet.to_vec()?,
            tx_info: Some(gw::DownlinkTxInfo {
                frequency: get_relay_frequency(&conf)?,
                power: conf.relay.tx_power,
                modulation: Some(helpers::data_rate_to_gw_modulation(
                    &conf.relay.data_rate,
                    false,
                )),
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

    backend::relay(&pl).await
}

async fn relay_downlink_lora_packet(pl: &gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    let conf = config::get();

    let mut tx_ack_items: Vec<gw::DownlinkTxAckItem> = pl
        .items
        .iter()
        .map(|_| gw::DownlinkTxAckItem {
            status: gw::TxAckStatus::Ignored.into(),
        })
        .collect();

    for (i, downlink_item) in pl.items.iter().enumerate() {
        let tx_info = downlink_item
            .tx_info
            .as_ref()
            .ok_or_else(|| anyhow!("tx_info is None"))?;
        let modulation = tx_info
            .modulation
            .as_ref()
            .ok_or_else(|| anyhow!("modulation is None"))?;
        let timing = tx_info
            .timing
            .as_ref()
            .ok_or_else(|| anyhow!("timing is None"))?;
        let delay = match &timing.parameters {
            Some(gw::timing::Parameters::Delay(v)) => v
                .delay
                .as_ref()
                .map(|v| v.seconds as u8)
                .unwrap_or_default(),
            _ => {
                return Err(anyhow!("Only Delay timing is supported"));
            }
        };

        let ctx = tx_info
            .context
            .get(CTX_PREFIX.len()..CTX_PREFIX.len() + 6)
            .ok_or_else(|| anyhow!("context does not contain enough bytes"))?;

        let packet = packets::RelayPacket {
            mhdr: packets::MHDR {
                payload_type: packets::PayloadType::Downlink,
                hop_count: 0,
            },
            payload: packets::Payload::Downlink(packets::DownlinkPayload {
                phy_payload: downlink_item.phy_payload.clone(),
                relay_id: {
                    let mut b: [u8; 4] = [0; 4];
                    b.copy_from_slice(&ctx[0..4]);
                    b
                },
                metadata: DownlinkMetadata {
                    uplink_id: {
                        let mut b: [u8; 2] = [0; 2];
                        b.copy_from_slice(&ctx[4..6]);
                        u16::from_be_bytes(b)
                    },
                    dr: helpers::modulation_to_dr(modulation)?,
                    frequency: tx_info.frequency,
                    delay,
                },
            }),
        };

        let pl = gw::DownlinkFrame {
            downlink_id: pl.downlink_id,
            items: vec![gw::DownlinkFrameItem {
                phy_payload: packet.to_vec()?,
                tx_info: Some(gw::DownlinkTxInfo {
                    frequency: get_relay_frequency(&conf)?,
                    power: conf.relay.tx_power,
                    modulation: Some(helpers::data_rate_to_gw_modulation(
                        &conf.relay.data_rate,
                        false,
                    )),
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

        match backend::relay(&pl).await {
            Ok(_) => {
                tx_ack_items[i].status = gw::TxAckStatus::Ok.into();
                break;
            }
            Err(e) => {
                warn!("Relay downlink failed, error: {}", e);
                tx_ack_items[i].status = gw::TxAckStatus::InternalError.into();
            }
        }
    }

    Ok(gw::DownlinkTxAck {
        gateway_id: pl.gateway_id.clone(),
        downlink_id: pl.downlink_id,
        items: tx_ack_items,
        ..Default::default()
    })
}

fn get_relay_frequency(conf: &Configuration) -> Result<u32> {
    if conf.relay.frequencies.is_empty() {
        return Err(anyhow!("No relay frequencies are configured"));
    }

    let mut relay_channel = RELAY_CHANNEL.lock().unwrap();
    *relay_channel += 1;

    if *relay_channel >= conf.relay.frequencies.len() {
        *relay_channel = 0;
    }

    Ok(conf.relay.frequencies[*relay_channel])
}

fn get_uplink_id() -> u16 {
    let mut uplink_id = UPLINK_ID.lock().unwrap();
    *uplink_id += 1;

    if *uplink_id > 4095 {
        *uplink_id = 0;
    }

    *uplink_id
}

fn store_uplink_context(ctx: &[u8]) -> u16 {
    let uplink_id = get_uplink_id();
    let mut uplink_ctx = UPLINK_CONTEXT.lock().unwrap();
    uplink_ctx.insert(uplink_id, ctx.to_vec());
    uplink_id
}
