use anyhow::Result;

use crate::config;
use chirpstack_api::gw;

pub fn frequency_to_chan(freq: u32) -> Result<u8> {
    let conf = config::get();
    for (i, f) in conf.mappings.channels.iter().enumerate() {
        if freq == *f {
            return Ok(i as u8);
        }
    }

    Err(anyhow!("Frequency {} does not map to a channel", freq))
}

pub fn chan_to_frequency(chan: u8) -> Result<u32> {
    let conf = config::get();
    conf.mappings
        .channels
        .get(chan as usize)
        .cloned()
        .ok_or_else(|| anyhow!("Channel {} does not map to a frequency", chan))
}

pub fn modulation_to_dr(modulation: &gw::Modulation) -> Result<u8> {
    let mod_params = modulation
        .parameters
        .as_ref()
        .ok_or_else(|| anyhow!("parameters must not be None"))?;

    let dr = match mod_params {
        gw::modulation::Parameters::Lora(v) => config::DataRate {
            modulation: config::Modulation::LORA,
            bandwidth: v.bandwidth,
            code_rate: Some(match v.code_rate() {
                gw::CodeRate::Cr45 => config::CodeRate::Cr45,
                gw::CodeRate::Cr46 => config::CodeRate::Cr46,
                gw::CodeRate::Cr47 => config::CodeRate::Cr47,
                gw::CodeRate::Cr48 => config::CodeRate::Cr48,
                gw::CodeRate::Cr38 => config::CodeRate::Cr38,
                gw::CodeRate::Cr26 => config::CodeRate::Cr26,
                gw::CodeRate::Cr14 => config::CodeRate::Cr14,
                gw::CodeRate::Cr16 => config::CodeRate::Cr16,
                gw::CodeRate::Cr56 => config::CodeRate::Cr56,
                gw::CodeRate::CrLi45 => config::CodeRate::CrLi45,
                gw::CodeRate::CrLi46 => config::CodeRate::CrLi46,
                gw::CodeRate::CrLi48 => config::CodeRate::CrLi48,
                gw::CodeRate::CrUndefined => {
                    return Err(anyhow!("code_rate is CrUndefined"));
                }
            }),
            spreading_factor: v.spreading_factor as u8,
            ..Default::default()
        },
        gw::modulation::Parameters::Fsk(v) => config::DataRate {
            modulation: config::Modulation::FSK,
            bitrate: v.datarate,
            ..Default::default()
        },
        gw::modulation::Parameters::LrFhss(_) => {
            return Err(anyhow!("LR-FHSS is not supported"));
        }
    };

    let conf = config::get();
    for (i, d) in conf.mappings.data_rates.iter().enumerate() {
        if dr == *d {
            return Ok(i as u8);
        }
    }

    Err(anyhow!(
        "Modulation: {:?} does not map to a data-rate",
        modulation
    ))
}

pub fn dr_to_modulation(dr: u8, ipol: bool) -> Result<gw::Modulation> {
    let conf = config::get();
    let dr = conf
        .mappings
        .data_rates
        .get(dr as usize)
        .ok_or_else(|| anyhow!("Data-rate {} does not map to a modulation", dr))?;

    Ok(data_rate_to_gw_modulation(dr, ipol))
}

pub fn data_rate_to_gw_modulation(dr: &config::DataRate, ipol: bool) -> gw::Modulation {
    match dr.modulation {
        config::Modulation::LORA => gw::Modulation {
            parameters: Some(gw::modulation::Parameters::Lora(gw::LoraModulationInfo {
                bandwidth: dr.bandwidth,
                spreading_factor: dr.spreading_factor as u32,
                code_rate: match dr.code_rate {
                    None => gw::CodeRate::CrUndefined,
                    Some(config::CodeRate::Cr45) => gw::CodeRate::Cr45,
                    Some(config::CodeRate::Cr46) => gw::CodeRate::Cr46,
                    Some(config::CodeRate::Cr47) => gw::CodeRate::Cr47,
                    Some(config::CodeRate::Cr48) => gw::CodeRate::Cr48,
                    Some(config::CodeRate::Cr38) => gw::CodeRate::Cr38,
                    Some(config::CodeRate::Cr26) => gw::CodeRate::Cr26,
                    Some(config::CodeRate::Cr14) => gw::CodeRate::Cr14,
                    Some(config::CodeRate::Cr16) => gw::CodeRate::Cr16,
                    Some(config::CodeRate::Cr56) => gw::CodeRate::Cr56,
                    Some(config::CodeRate::CrLi45) => gw::CodeRate::CrLi45,
                    Some(config::CodeRate::CrLi46) => gw::CodeRate::CrLi46,
                    Some(config::CodeRate::CrLi48) => gw::CodeRate::CrLi48,
                }
                .into(),
                polarization_inversion: ipol,
                ..Default::default()
            })),
        },
        config::Modulation::FSK => gw::Modulation {
            parameters: Some(gw::modulation::Parameters::Fsk(gw::FskModulationInfo {
                frequency_deviation: dr.bitrate / 2,
                datarate: dr.bitrate,
            })),
        },
    }
}

// This either returns the index matching the exact tx_power, or an index which
// holds the closest value, but lower.
pub fn tx_power_to_index(tx_power: i32) -> Result<u8> {
    let conf = config::get();
    let mut out: Option<u8> = None;

    for (i, p) in conf.mappings.tx_power.iter().enumerate() {
        if *p <= tx_power {
            match &mut out {
                Some(v) => {
                    if conf.mappings.tx_power[*v as usize] < tx_power {
                        *v = i as u8;
                    }
                }
                None => {
                    out = Some(i as u8);
                }
            }
        }
    }

    out.ok_or_else(|| anyhow!("No TX Power equal or lower than: {}", tx_power))
}

pub fn index_to_tx_power(tx_power: u8) -> Result<i32> {
    let conf = config::get();
    conf.mappings
        .tx_power
        .get(tx_power as usize)
        .cloned()
        .ok_or_else(|| anyhow!("TX Power index {} does not exist", tx_power))
}

pub fn tx_ack_to_err(tx_ack: &gw::DownlinkTxAck) -> Result<()> {
    let tx_ack_ok: Vec<gw::DownlinkTxAckItem> = tx_ack
        .items
        .iter()
        .filter(|v| v.status() == gw::TxAckStatus::Ok)
        .cloned()
        .collect();

    if tx_ack_ok.is_empty() {
        Err(anyhow!(
            "Tx Ack error: {}",
            tx_ack
                .items
                .last()
                .cloned()
                .unwrap_or_default()
                .status()
                .as_str_name()
        ))
    } else {
        Ok(())
    }
}
