use std::sync::{Arc, Mutex};

use anyhow::Result;
use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use log::{error, info, trace, warn};
use once_cell::sync::OnceCell;
use tokio::task;

use crate::backend;
use crate::config::Configuration;
use crate::relay;

static EVENT_SOCKET: OnceCell<Mutex<zmq::Socket>> = OnceCell::new();

pub enum Command {
    Timeout,
    Unknown(String, Vec<u8>),
    Downlink(gw::DownlinkFrame),
    GatewayID,
    Configuration(gw::GatewayConfiguration),
}

pub fn setup(conf: &Configuration) -> Result<()> {
    if !conf.relay.border_gateway {
        return Ok(());
    }

    info!(
        "Setting up Concentratord proxy API, event_bind: {}, command_bind: {}",
        conf.relay.proxy_api.event_bind, conf.relay.proxy_api.command_bind
    );

    let zmq_ctx = zmq::Context::new();
    let sock = zmq_ctx.socket(zmq::PUB)?;
    sock.bind(&conf.relay.proxy_api.event_bind)?;
    EVENT_SOCKET
        .set(Mutex::new(sock))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    let sock = zmq_ctx.socket(zmq::REP)?;
    sock.bind(&conf.relay.proxy_api.command_bind)?;

    tokio::spawn({
        async move {
            command_loop(sock).await;
        }
    });

    Ok(())
}

pub async fn send_uplink(pl: &gw::UplinkFrame) -> Result<()> {
    info!(
        "Sending uplink event, uplink_id: {}",
        pl.rx_info
            .as_ref()
            .ok_or_else(|| anyhow!("rx_info is None"))?
            .uplink_id
    );

    task::spawn_blocking({
        let b = pl.encode_to_vec();

        move || -> Result<()> {
            let event_sock = EVENT_SOCKET
                .get()
                .ok_or_else(|| anyhow!("Proxy API is not (yet) initialized"))?;

            let sock = event_sock.lock().unwrap();
            sock.send("up", zmq::SNDMORE)?;
            sock.send(b, 0)?;

            Ok(())
        }
    })
    .await?
}

pub async fn send_stats(pl: &gw::GatewayStats) -> Result<()> {
    info!("Sending stats event");

    task::spawn_blocking({
        let b = pl.encode_to_vec();

        move || -> Result<()> {
            let event_sock = EVENT_SOCKET
                .get()
                .ok_or_else(|| anyhow!("Proxy API is not (yet) initialized"))?;

            let sock = event_sock.lock().unwrap();
            sock.send("stats", zmq::SNDMORE)?;
            sock.send(b, 0)?;

            Ok(())
        }
    })
    .await?
}

async fn command_loop(rep_sock: zmq::Socket) {
    trace!("Starting command loop");
    let rep_sock = Arc::new(Mutex::new(rep_sock));

    loop {
        let cmd = match read_command(rep_sock.clone()).await {
            Ok(v) => v,
            Err(err) => {
                error!("Receive command error, error: {}", err);
                continue;
            }
        };

        let resp = match cmd {
            Command::Timeout => continue,
            Command::Unknown(_, _) => Vec::new(),
            Command::Configuration(v) => {
                info!("Configuration command received, version: {}", v.version);

                if let Err(e) = backend::send_gateway_configuration(&v).await {
                    error!("Send gateway configuration error: {}", e);
                }
                Vec::new()
            }
            Command::Downlink(v) => {
                info!("Downlink command received, downlink_id: {}", v.downlink_id);

                match relay::handle_downlink(v).await {
                    Ok(v) => v.encode_to_vec(),
                    Err(e) => {
                        error!("Handle downlink error: {}", e);
                        Vec::new()
                    }
                }
            }
            Command::GatewayID => {
                info!("Get gateway id command received");

                match backend::get_gateway_id() {
                    Ok(v) => v.to_vec(),
                    Err(e) => {
                        error!("Get gateway ID error: {}", e);
                        Vec::new()
                    }
                }
            }
        };

        let resp = task::spawn_blocking({
            let rep_sock = rep_sock.clone();

            move || -> Result<()> {
                rep_sock
                    .lock()
                    .unwrap()
                    .send(resp, 0)
                    .map_err(anyhow::Error::new)
            }
        })
        .await;

        if let Err(e) = &resp {
            error!("Sending to ZMQ REP socket error: {}", e);
        }

        if let Ok(Err(e)) = &resp {
            error!("Sending to ZMQ REP socket error: {}", e);
        }
    }
}

async fn read_command(rep_sock: Arc<Mutex<zmq::Socket>>) -> Result<Command> {
    trace!("Reading next command from zmq socket");

    task::spawn_blocking({
        move || -> Result<Command> {
            let rep_sock = rep_sock.lock().unwrap();

            let mut items = [rep_sock.as_poll_item(zmq::POLLIN)];
            zmq::poll(&mut items, 100)?;
            if !items[0].is_readable() {
                return Ok(Command::Timeout);
            }

            let msg = rep_sock.recv_multipart(0)?;

            if msg.len() != 2 {
                return Err(anyhow!("Command must have two frames"));
            }

            let command = String::from_utf8(msg[0].clone())?;

            match command.as_str() {
                "down" => gw::DownlinkFrame::decode(&*msg[1])
                    .map(Command::Downlink)
                    .map_err(anyhow::Error::new),
                "config" => gw::GatewayConfiguration::decode(&*msg[1])
                    .map(Command::Configuration)
                    .map_err(anyhow::Error::new),
                "gateway_id" => Ok(Command::GatewayID),
                _ => Err(anyhow!("Unknown command: {}", command)),
            }
        }
    })
    .await?
}
