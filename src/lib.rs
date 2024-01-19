#[macro_use]
extern crate anyhow;

use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;

pub mod backend;
pub mod cmd;
pub mod config;
pub mod helpers;
pub mod logging;
pub mod packets;
pub mod proxy;
pub mod relay;

// pub static ZMQ_CONTEXT: Lazy<Arc<Mutex<zmq::Context>>> =
//     Lazy::new(|| Arc::new(Mutex::new(zmq::Context::new())));
