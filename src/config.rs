use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Configuration {
    pub logging: Logging,
    pub relay: Relay,
    pub backend: Backend,
    pub channels: Vec<Channel>,
    pub data_rates: Vec<DataRate>,
}

impl Configuration {
    pub fn get(filenames: &[String]) -> Result<Configuration> {
        let mut content = String::new();
        for file_name in filenames {
            content.push_str(&fs::read_to_string(file_name)?);
        }
        Ok(toml::from_str(&content)?)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Logging {
    pub level: String,
    pub log_to_syslog: bool,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            level: "info".into(),
            log_to_syslog: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Relay {
    pub frequencies: Vec<u32>,
    pub data_rate: DataRate,
    pub proxy_api: ProxyApi,
    pub filters: Filters,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Backend {
    pub concentratord: Concentratord,
    pub relay_concentratord: Concentratord,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Concentratord {
    pub event_url: String,
    pub command_url: String,
}

impl Default for Concentratord {
    fn default() -> Self {
        Concentratord {
            event_url: "ipc:///tmp/concentratord_event".into(),
            command_url: "ipc:///tmp/concentratord_command".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyApi {
    pub event_bind: String,
    pub command_bind: String,
}

impl Default for ProxyApi {
    fn default() -> Self {
        ProxyApi {
            event_bind: "ipc:///tmp/chirpstack_gateway_relay_event".into(),
            command_bind: "ipc:///tmp/chirpstack_gateway_relay_command".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Filters {
    pub dev_addr_prefixes: Vec<lrwn_filters::DevAddrPrefix>,
    pub join_eui_prefixes: Vec<lrwn_filters::EuiPrefix>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Channel {
    pub frequency: u32,
    pub min_dr: u8,
    pub max_dr: u8,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DataRate {
    spreading_factor: u8,
    bandwidth: u32,
    coding_rate: String,
    bitrate: u32,
}
