use anyhow::Result;

use crate::config;
use chirpstack_api::gw;

pub fn frequency_to_chan(freq: u32) -> Result<u8> {
    let conf = config::get();
    for (i, f) in conf.channels.iter().enumerate() {
        if freq == *f {
            return Ok(i as u8);
        }
    }

    Err(anyhow!("Frequency {} does not map to a channel", freq))
}

pub fn chan_to_frequency(chan: u8) -> Result<u32> {
    let conf = config::get();
    conf.channels
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
    for (i, d) in conf.data_rates.iter().enumerate() {
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
