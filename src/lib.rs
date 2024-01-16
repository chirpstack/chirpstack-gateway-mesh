#[macro_use]
extern crate anyhow;

use std::sync::Mutex;

use once_cell::sync::Lazy;

pub mod backend;
pub mod cmd;
pub mod config;
pub mod helpers;
pub mod logging;
pub mod packets;
pub mod proxy;
pub mod relay;

pub static ZMQ_CONTEXT: Lazy<Mutex<zmq::Context>> = Lazy::new(|| Mutex::new(zmq::Context::new()));
