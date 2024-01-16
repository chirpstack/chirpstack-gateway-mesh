use std::sync::Mutex;

use anyhow::Result;
use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use log::{info, warn};
use once_cell::sync::OnceCell;
use tokio::task;

use crate::config::Configuration;
use crate::ZMQ_CONTEXT;

static EVENT_SOCKET: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();
static COMMAND_SOCKET: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();

pub fn setup(conf: &Configuration) -> Result<()> {
    if !conf.relay.border_gateway {
        return Ok(());
    }

    info!(
        "Setting up Concentratord proxy API, event_bind: {}, command_bind: {}",
        conf.relay.proxy_api.event_bind, conf.relay.proxy_api.command_bind
    );

    let zmq_ctx = ZMQ_CONTEXT.lock().unwrap();
    let sock = zmq_ctx.socket(zmq::PUB)?;
    sock.bind(&conf.relay.proxy_api.event_bind)?;
    EVENT_SOCKET
        .set(Mutex::new(sock))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    let sock = zmq_ctx.socket(zmq::REP)?;
    sock.bind(&conf.relay.proxy_api.command_bind)?;
    COMMAND_SOCKET
        .set(Mutex::new(sock))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

pub async fn send_uplink(pl: &gw::UplinkFrame) -> Result<()> {
    task::spawn_blocking({
        let b = pl.encode_to_vec();

        move || -> Result<()> {
            let event_sock = match EVENT_SOCKET.get() {
                Some(v) => v,
                None => {
                    warn!("Proxy API is not (yet) initialized");
                    return Ok(());
                }
            };

            let sock = event_sock.lock().unwrap();
            sock.send("up", zmq::SNDMORE).unwrap();
            sock.send(b, 0).unwrap();

            Ok(())
        }
    })
    .await?
}

pub async fn send_stats(pl: &gw::GatewayStats) -> Result<()> {
    task::spawn_blocking({
        let b = pl.encode_to_vec();

        move || -> Result<()> {
            let event_sock = match EVENT_SOCKET.get() {
                Some(v) => v,
                None => {
                    warn!("Proxy API is not (yet) initialized");
                    return Ok(());
                }
            };

            let sock = event_sock.lock().unwrap();
            sock.send("stats", zmq::SNDMORE).unwrap();
            sock.send(b, 0).unwrap();

            Ok(())
        }
    })
    .await?
}
