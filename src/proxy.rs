use std::thread;

use anyhow::Result;
use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use log::{error, info, trace};
use once_cell::sync::OnceCell;
use tokio::sync::{mpsc, oneshot};

use crate::backend;
use crate::config::Configuration;
use crate::helpers;
use crate::mesh;

static EVENT_CHAN: OnceCell<EventChannel> = OnceCell::new();

type Event = (String, Vec<u8>);
type Command = ((String, Vec<u8>), oneshot::Sender<Vec<u8>>);
type EventChannel = mpsc::UnboundedSender<Event>;
type CommandChannel = mpsc::UnboundedReceiver<Command>;

pub async fn setup(conf: &Configuration) -> Result<()> {
    if !conf.mesh.border_gateway {
        return Ok(());
    }

    info!(
        "Setting up Concentratord proxy API, event_bind: {}, command_bind: {}",
        conf.mesh.proxy_api.event_bind, conf.mesh.proxy_api.command_bind
    );

    // Setup ZMQ event.

    // As the zmq::Context can't be shared between threads, we use a channel.
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();

    // Spawn the zmq event handler to a dedicated thread.
    thread::spawn({
        let event_bind = conf.mesh.proxy_api.event_bind.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let sock = zmq_ctx.socket(zmq::PUB).unwrap();
            sock.bind(&event_bind).unwrap();

            while let Some(event) = event_rx.blocking_recv() {
                sock.send(&event.0, zmq::SNDMORE).unwrap();
                sock.send(&event.1, 0).unwrap();
            }
        }
    });

    // Set event channel.

    EVENT_CHAN
        .set(event_tx)
        .map_err(|e| anyhow!("OnceCell error: {:?}", e))?;

    // Setup ZMQ command.

    let (command_tx, command_rx) = mpsc::unbounded_channel::<Command>();

    // Spawn the zmq command handler to a dedicated thread.
    thread::spawn({
        let command_bind = conf.mesh.proxy_api.command_bind.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let mut sock = zmq_ctx.socket(zmq::REP).unwrap();
            sock.bind(&command_bind).unwrap();

            loop {
                match receive_zmq_command(&mut sock) {
                    Ok(v) => {
                        let (resp_tx, resp_rx) = oneshot::channel::<Vec<u8>>();
                        command_tx.send(((v.0, v.1), resp_tx)).unwrap();

                        match resp_rx.blocking_recv() {
                            Ok(v) => sock.send(&v, 0).unwrap(),
                            Err(e) => {
                                error!("Receive command response error, error: {}", e);
                                sock.send(vec![], 0).unwrap();
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error receiving ZMQ command: {}", e);
                        sock.send(vec![], 0).unwrap();
                    }
                }
            }
        }
    });

    // Spawn command handler.
    tokio::spawn({
        async move {
            command_loop(command_rx).await;
        }
    });

    Ok(())
}

pub async fn send_uplink(pl: &gw::UplinkFrame) -> Result<()> {
    info!("Sending uplink event - {}", helpers::format_uplink(pl)?);

    let event_chan = EVENT_CHAN
        .get()
        .ok_or_else(|| anyhow!("EVENT_CHAN is not set"))?;

    event_chan.send(("up".to_string(), pl.encode_to_vec()))?;

    Ok(())
}

pub async fn send_stats(pl: &gw::GatewayStats) -> Result<()> {
    info!("Sending gateway stats event");

    let event_chan = EVENT_CHAN
        .get()
        .ok_or_else(|| anyhow!("EVENT_CHAN is not set"))?;

    event_chan.send(("stats".to_string(), pl.encode_to_vec()))?;

    Ok(())
}

pub async fn send_mesh_heartbeat(pl: &gw::MeshHeartbeat) -> Result<()> {
    info!("Sending mesh heartbeat event");

    let event_chan = EVENT_CHAN
        .get()
        .ok_or_else(|| anyhow!("EVENT_CHAN is not set"))?;

    event_chan.send(("mesh_heartbeat".to_string(), pl.encode_to_vec()))?;

    Ok(())
}

async fn command_loop(mut command_rx: CommandChannel) {
    trace!("Starting command loop");

    while let Some(cmd) = command_rx.recv().await {
        match handle_command(&cmd).await {
            Ok(v) => {
                _ = cmd.1.send(v);
            }
            Err(e) => {
                error!("Handle command error: {}", e);
                let _ = cmd.1.send(vec![]);
            }
        }
    }

    error!("Command loop has been interrupted");
}

async fn handle_command(cmd: &Command) -> Result<Vec<u8>> {
    Ok(match cmd.0 .0.as_str() {
        "config" => {
            let pl = gw::GatewayConfiguration::decode(cmd.0 .1.as_slice())?;
            info!("Configuration command received, version: {}", pl.version);
            backend::send_gateway_configuration(&pl).await?;
            Vec::new()
        }
        "down" => {
            let pl = gw::DownlinkFrame::decode(cmd.0 .1.as_slice())?;
            info!(
                "Downlink command received - {}",
                helpers::format_downlink(&pl)?
            );
            mesh::handle_downlink(pl).await.map(|v| v.encode_to_vec())?
        }
        "gateway_id" => {
            info!("Get gateway id command received");
            backend::get_gateway_id().await.map(|v| v.to_vec())?
        }
        _ => {
            return Err(anyhow!("Unexpected command: {}", cmd.0 .0));
        }
    })
}

fn receive_zmq_command(sock: &mut zmq::Socket) -> Result<(String, Vec<u8>)> {
    let msg = sock.recv_multipart(0).unwrap();
    if msg.len() != 2 {
        return Err(anyhow!("Command must have 2 frames"));
    }

    let cmd = String::from_utf8(msg[0].to_vec())?;
    let b = msg[1].to_vec();

    Ok((cmd, b))
}
