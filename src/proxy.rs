use std::sync::OnceLock;
use std::thread;

use anyhow::Result;
use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use log::{error, info, trace};
use tokio::sync::{mpsc, oneshot};

use crate::backend;
use crate::config::Configuration;
use crate::helpers;
use crate::mesh;

static EVENT_CHAN: OnceLock<EventChannel> = OnceLock::new();

type EventChannel = mpsc::UnboundedSender<gw::Event>;
type Command = (gw::Command, oneshot::Sender<Vec<u8>>);
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
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<gw::Event>();

    // Spawn the zmq event handler to a dedicated thread.
    thread::spawn({
        let event_bind = conf.mesh.proxy_api.event_bind.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let sock = zmq_ctx.socket(zmq::PUB).unwrap();
            sock.bind(&event_bind).unwrap();

            while let Some(event) = event_rx.blocking_recv() {
                sock.send(&event.encode_to_vec(), 0).unwrap();
            }
        }
    });

    // Set event channel.
    EVENT_CHAN
        .set(event_tx)
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

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
                        command_tx.send((v, resp_tx)).unwrap();

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

pub async fn send_event(pl: gw::Event) -> Result<()> {
    info!("Sending event");

    let event_chan = EVENT_CHAN
        .get()
        .ok_or_else(|| anyhow!("EVENT_CHAN is not set"))?;

    event_chan.send(pl)?;

    Ok(())
}

async fn command_loop(mut command_rx: CommandChannel) {
    trace!("Starting command loop");

    while let Some(cmd) = command_rx.recv().await {
        match handle_command(cmd.0).await {
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

async fn handle_command(cmd: gw::Command) -> Result<Vec<u8>> {
    Ok(match cmd.command {
        Some(gw::command::Command::SetGatewayConfiguration(v)) => {
            info!("Configuration command received, version: {}", v.version);
            backend::send_gateway_configuration(v).await?;
            Vec::new()
        }
        Some(gw::command::Command::SendDownlinkFrame(v)) => {
            info!(
                "Downlink command received - {}",
                helpers::format_downlink(&v)?
            );
            mesh::handle_downlink(v).await.map(|v| v.encode_to_vec())?
        }
        Some(gw::command::Command::GetGatewayId(_)) => {
            info!("Get gateway id command received");
            gw::GetGatewayIdResponse {
                gateway_id: hex::encode(&backend::get_gateway_id().await.unwrap_or_default()),
            }
            .encode_to_vec()
        }
        Some(gw::command::Command::Mesh(v)) => {
            info!("Mesh command received");
            mesh::send_mesh_command(v).await?;
            Vec::new()
        }
        _ => return Err(anyhow!("Unexpected command: {:?}", cmd.command)),
    })
}

fn receive_zmq_command(sock: &mut zmq::Socket) -> Result<gw::Command> {
    let b = sock.recv_bytes(0)?;
    let cmd = gw::Command::decode(b.as_slice())?;
    Ok(cmd)
}
