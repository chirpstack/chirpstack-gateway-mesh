use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use once_cell::sync::OnceCell;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::aes128::Aes128Key;

static CONFIG: OnceCell<Mutex<Arc<Configuration>>> = OnceCell::new();

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Configuration {
    pub logging: Logging,
    pub mesh: Mesh,
    pub backend: Backend,
    pub mappings: Mappings,
}

impl Configuration {
    pub fn load(filenames: &[String]) -> Result<()> {
        let mut content = String::new();
        for file_name in filenames {
            content.push_str(&fs::read_to_string(file_name)?);
        }

        let conf: Configuration = toml::from_str(&content)?;
        set(conf)
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

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Mesh {
    pub signing_key: Aes128Key,
    #[serde(with = "humantime_serde")]
    pub heartbeat_interval: Duration,
    pub frequencies: Vec<u32>,
    pub data_rate: DataRate,
    pub tx_power: i32,
    pub proxy_api: ProxyApi,
    pub filters: Filters,
    pub border_gateway: bool,
    pub border_gateway_ignore_direct_uplinks: bool,
    pub max_hop_count: u8,
}

impl Default for Mesh {
    fn default() -> Self {
        Mesh {
            signing_key: Aes128Key::null(),
            heartbeat_interval: Duration::from_secs(300),
            frequencies: vec![868100000, 868300000, 868500000],
            data_rate: DataRate {
                modulation: Modulation::LORA,
                spreading_factor: 7,
                bandwidth: 125000,
                code_rate: Some(CodeRate::Cr45),
                bitrate: 0,
            },
            tx_power: 16,
            proxy_api: ProxyApi::default(),
            filters: Filters::default(),
            border_gateway: false,
            border_gateway_ignore_direct_uplinks: false,
            max_hop_count: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Backend {
    pub concentratord: Concentratord,
    pub mesh_concentratord: Concentratord,
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
            event_bind: "ipc:///tmp/gateway_relay_event".into(),
            command_bind: "ipc:///tmp/gateway_relay_command".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Filters {
    pub dev_addr_prefixes: Vec<lrwn_filters::DevAddrPrefix>,
    pub join_eui_prefixes: Vec<lrwn_filters::EuiPrefix>,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Mappings {
    pub channels: Vec<u32>,
    pub tx_power: Vec<i32>,
    pub data_rates: Vec<DataRate>,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct DataRate {
    pub modulation: Modulation,
    pub spreading_factor: u8,
    pub bandwidth: u32,
    pub code_rate: Option<CodeRate>,
    pub bitrate: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
pub enum Modulation {
    #[default]
    LORA,
    FSK,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CodeRate {
    Cr45,
    Cr46,
    Cr47,
    Cr48,
    Cr38,
    Cr26,
    Cr14,
    Cr16,
    Cr56,
    CrLi45,
    CrLi46,
    CrLi48,
}

impl Serialize for CodeRate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            CodeRate::Cr45 => serializer.serialize_str("4/5"),
            CodeRate::Cr46 => serializer.serialize_str("4/6"),
            CodeRate::Cr47 => serializer.serialize_str("4/7"),
            CodeRate::Cr48 => serializer.serialize_str("4/8"),
            CodeRate::Cr38 => serializer.serialize_str("3/8"),
            CodeRate::Cr26 => serializer.serialize_str("2/6"),
            CodeRate::Cr14 => serializer.serialize_str("1/4"),
            CodeRate::Cr16 => serializer.serialize_str("1/6"),
            CodeRate::Cr56 => serializer.serialize_str("5/6"),
            CodeRate::CrLi45 => serializer.serialize_str("4/5LI"),
            CodeRate::CrLi46 => serializer.serialize_str("4/6LI"),
            CodeRate::CrLi48 => serializer.serialize_str("4/5LI"),
        }
    }
}

impl<'de> Deserialize<'de> for CodeRate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "4/5" => CodeRate::Cr45,
            "4/6" | "2/3" => CodeRate::Cr46,
            "4/7" => CodeRate::Cr47,
            "4/8" | "2/4" | "1/2" => CodeRate::Cr48,
            "3/8" => CodeRate::Cr38,
            "2/6" | "1/3" => CodeRate::Cr26,
            "1/4" => CodeRate::Cr14,
            "1/6" => CodeRate::Cr16,
            "5/6" => CodeRate::Cr56,
            "4/5LI" => CodeRate::CrLi45,
            "4/6LI" => CodeRate::CrLi46,
            "4/8LI" => CodeRate::CrLi48,
            _ => return Err(Error::custom(format!("Unexpected code_rate: {}", s))),
        })
    }
}

pub fn set(c: Configuration) -> Result<()> {
    CONFIG
        .set(Mutex::new(Arc::new(c)))
        .map_err(|_| anyhow!("Set OnceCell error"))
}

pub fn get() -> Arc<Configuration> {
    let conf = CONFIG
        .get()
        .ok_or_else(|| anyhow!("OnceCell is not set"))
        .unwrap();

    conf.lock().unwrap().clone()
}
