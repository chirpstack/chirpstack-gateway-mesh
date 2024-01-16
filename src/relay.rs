use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use chirpstack_api::gw;
use once_cell::sync::Lazy;
use rand::random;

use crate::{
    backend,
    config::{self, Configuration},
    helpers,
    packets::{Packet, Payload, PayloadType, RelayPacket, UplinkMetadata, UplinkPayload, MHDR},
    proxy,
};

static RELAY_CHANNEL: Mutex<usize> = Mutex::new(0);
static UPLINK_ID: Mutex<u16> = Mutex::new(0);
static UPLINK_CONTEXT: Lazy<Mutex<HashMap<u16, Vec<u8>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn handle_uplink(border_gateway: bool, pl: gw::UplinkFrame) -> Result<()> {
    let packet = Packet::from_slice(&pl.phy_payload)?;

    match packet {
        Packet::Relay(v) => match border_gateway {
            true => proxy_uplink_relay_packet(&pl, v).await?,
            false => relay_relay_packet(&pl, v).await?,
        },
        Packet::Lora(_) => match border_gateway {
            true => proxy_uplink_lora_packet(&pl).await?,
            false => relay_uplink_lora_packet(&pl).await?,
        },
    }

    Ok(())
}

pub async fn handle_stats(border_gateway: bool, pl: gw::GatewayStats) -> Result<()> {
    if !border_gateway {
        return Ok(());
    }
    proxy::send_stats(&pl).await
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
        rx_info
            .metadata
            .insert("hop_count".to_string(), packet.mhdr.hop_count.to_string());
        rx_info
            .metadata
            .insert("relay_id".to_string(), hex::encode(&relay_pl.relay_id));

        // Set RSSI and SNR.
        rx_info.snr = relay_pl.metadata.snr.into();
        rx_info.rssi = relay_pl.metadata.rssi.into();

        // Set context.
        rx_info.context = relay_pl.metadata.uplink_id.to_be_bytes().to_vec();
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
                    true,
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
                dr: helpers::modulation_to_dr(&modulation)?,
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
                    true,
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
