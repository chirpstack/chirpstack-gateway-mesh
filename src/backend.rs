use std::sync::OnceLock;
use std::thread;

use anyhow::Result;
use chirpstack_api::prost::Message;
use log::{debug, error, info, trace};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::config::Configuration;
use crate::{helpers, mesh, proxy};
use chirpstack_api::gw;

static GATEWAY_ID: OnceLock<Mutex<[u8; 8]>> = OnceLock::new();
static RELAY_ID: OnceLock<Mutex<[u8; 4]>> = OnceLock::new();

static CONCENTRATORD_CMD_CHAN: OnceLock<CommandChannel> = OnceLock::new();
static MESH_CONCENTRATORD_CMD_CHAN: OnceLock<CommandChannel> = OnceLock::new();

type Command = (gw::Command, oneshot::Sender<Result<Vec<u8>>>);
type CommandChannel = mpsc::UnboundedSender<Command>;

pub async fn setup(conf: &Configuration) -> Result<()> {
    setup_concentratord(conf).await?;
    setup_mesh_concentratord(conf).await?;
    Ok(())
}

async fn setup_concentratord(conf: &Configuration) -> Result<()> {
    info!(
        "Setting up Concentratord backend, event_url: {}, command_url: {}",
        conf.backend.concentratord.event_url, conf.backend.concentratord.command_url
    );

    // Setup ZMQ command.

    // As the zmq::Context can't be shared between threads, we use a channel.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();

    // Spawn the zmq command handler to a dedicated thread.
    thread::spawn({
        let command_url = conf.backend.concentratord.command_url.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let mut sock = zmq_ctx.socket(zmq::REQ).unwrap();
            sock.connect(&command_url).unwrap();

            while let Some(cmd) = cmd_rx.blocking_recv() {
                let resp = send_zmq_command(&mut sock, &cmd.0);
                cmd.1.send(resp).unwrap();
            }

            error!("Concentratord command loop has been interrupted");
        }
    });

    // Read Gateway ID.

    trace!("Reading Gateway ID");
    let mut gateway_id: [u8; 8] = [0; 8];
    let (gateway_id_tx, gateway_id_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_tx.send((
        gw::Command {
            command: Some(gw::command::Command::GetGatewayId(
                gw::GetGatewayIdRequest {},
            )),
        },
        gateway_id_tx,
    ))?;
    let resp = gateway_id_rx.await??;

    let resp = gw::GetGatewayIdResponse::decode(resp.as_slice())?;
    gateway_id.copy_from_slice(&hex::decode(&resp.gateway_id)?);
    info!("Retrieved Gateway ID: {}", resp.gateway_id);
    GATEWAY_ID
        .set(Mutex::new(gateway_id))
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Set CMD channel.
    CONCENTRATORD_CMD_CHAN
        .set(cmd_tx)
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Setup ZMQ event.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<gw::Event>();

    // Spawn the zmq event handler to a dedicated thread.
    thread::spawn({
        let event_url = conf.backend.concentratord.event_url.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let mut sock = zmq_ctx.socket(zmq::SUB).unwrap();
            sock.connect(&event_url).unwrap();
            sock.set_subscribe("".as_bytes()).unwrap();

            loop {
                match receive_zmq_event(&mut sock) {
                    Ok(v) => event_tx.send(v).unwrap(),
                    Err(e) => {
                        error!("Error receiving ZMQ event, error: {}", e);
                    }
                }
            }
        }
    });

    // Spawn event handler.
    tokio::spawn({
        let border_gateway = conf.mesh.border_gateway;
        let border_gateway_ignore_direct_uplinks = conf.mesh.border_gateway_ignore_direct_uplinks;
        let filters = lrwn_filters::Filters {
            dev_addr_prefixes: conf.mesh.filters.dev_addr_prefixes.clone(),
            join_eui_prefixes: conf.mesh.filters.join_eui_prefixes.clone(),
            lorawan_only: conf.mesh.filters.lorawan_only,
        };

        async move {
            event_loop(
                border_gateway,
                border_gateway_ignore_direct_uplinks,
                event_rx,
                filters,
            )
            .await;
        }
    });

    Ok(())
}

async fn setup_mesh_concentratord(conf: &Configuration) -> Result<()> {
    info!(
        "Setting up Mesh Concentratord backend, event_url: {}, command_url: {}",
        conf.backend.mesh_concentratord.event_url, conf.backend.mesh_concentratord.command_url
    );

    // Setup ZMQ command.

    // As the zmq::Context can't be shared between threads, we use a channel.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();

    // Spawn the zmq command handler to a dedicated thread.
    thread::spawn({
        let command_url = conf.backend.mesh_concentratord.command_url.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let mut sock = zmq_ctx.socket(zmq::REQ).unwrap();
            sock.connect(&command_url).unwrap();

            while let Some(cmd) = cmd_rx.blocking_recv() {
                let resp = send_zmq_command(&mut sock, &cmd.0);
                cmd.1.send(resp).unwrap();
            }

            error!("Mesh Concentratord command loop has been interrupted");
        }
    });

    // Read Relay ID.
    trace!("Reading Gateway ID");

    let (gateway_id_tx, gateway_id_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_tx.send((
        gw::Command {
            command: Some(gw::command::Command::GetGatewayId(
                gw::GetGatewayIdRequest {},
            )),
        },
        gateway_id_tx,
    ))?;
    let resp = gateway_id_rx.await??;
    let resp = gw::GetGatewayIdResponse::decode(resp.as_slice())?;
    info!("Retrieved Gateway ID: {}", resp.gateway_id);

    let mut relay_id: [u8; 4] = [0; 4];
    if conf.mesh.relay_id.is_empty() {
        relay_id.copy_from_slice(&hex::decode(&resp.gateway_id)?[4..]);
    } else {
        info!("Using relay_id from configuration file");
        let b = hex::decode(&conf.mesh.relay_id)?;
        if b.len() != 4 {
            return Err(anyhow!("relay_id must be exactly 4 bytes!"));
        }
        relay_id.copy_from_slice(&b);
    }
    RELAY_ID
        .set(Mutex::new(relay_id))
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // set CMD channel.

    MESH_CONCENTRATORD_CMD_CHAN
        .set(cmd_tx)
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Setup ZMQ event.

    let (event_tx, event_rx) = mpsc::unbounded_channel::<gw::Event>();

    // Spawn the zmq event handler to a dedicated thread;
    thread::spawn({
        let event_url = conf.backend.mesh_concentratord.event_url.clone();

        move || {
            let zmq_ctx = zmq::Context::new();
            let mut sock = zmq_ctx.socket(zmq::SUB).unwrap();
            sock.connect(&event_url).unwrap();
            sock.set_subscribe("".as_bytes()).unwrap();

            loop {
                match receive_zmq_event(&mut sock) {
                    Ok(v) => event_tx.send(v).unwrap(),
                    Err(e) => {
                        error!("Error receiving ZMQ event, error: {}", e);
                    }
                }
            }
        }
    });

    // Spawn event handler.
    tokio::spawn({
        let border_gateway = conf.mesh.border_gateway;

        async move {
            mesh_event_loop(border_gateway, event_rx).await;
        }
    });

    Ok(())
}

async fn event_loop(
    border_gateway: bool,
    border_gateway_ignore_direct_uplinks: bool,
    mut event_rx: mpsc::UnboundedReceiver<gw::Event>,
    filters: lrwn_filters::Filters,
) {
    trace!("Starting event loop");
    while let Some(event) = event_rx.recv().await {
        if let Err(e) = handle_event_msg(
            border_gateway,
            border_gateway_ignore_direct_uplinks,
            event,
            &filters,
        )
        .await
        {
            error!("Handle event error: {}", e);
            continue;
        }
    }
}

async fn mesh_event_loop(border_gateway: bool, mut event_rx: mpsc::UnboundedReceiver<gw::Event>) {
    trace!("Starting mesh event loop");
    while let Some(event) = event_rx.recv().await {
        if let Err(e) = handle_mesh_event_msg(border_gateway, event).await {
            error!("Handle mesh event error: {}", e);
            continue;
        }
    }
}

async fn handle_event_msg(
    border_gateway: bool,
    border_gateway_ignore_direct_uplinks: bool,
    event: gw::Event,
    filters: &lrwn_filters::Filters,
) -> Result<()> {
    trace!("Handling event, event: {:?}", event,);

    match &event.event {
        Some(gw::event::Event::UplinkFrame(v)) => {
            if let Some(rx_info) = &v.rx_info {
                // Filter out frames with invalid CRC.
                if rx_info.crc_status() != gw::CrcStatus::CrcOk {
                    debug!(
                        "Discarding uplink, CRC != OK, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Filter out proprietary payloads.
                if v.phy_payload.first().cloned().unwrap_or_default() & 0xe0 == 0xe0 {
                    debug!(
                        "Discarding proprietary uplink, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Ignore direct uplinks.
                if border_gateway_ignore_direct_uplinks {
                    debug!(
                        "Discarding direct uplink because of border_gateway_ignore_direct_uplinks setting, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Filter uplinks based on DevAddr and JoinEUI filters.
                if !lrwn_filters::matches(&v.phy_payload, filters) {
                    debug!(
                        "Discarding uplink because of dev_addr and join_eui filters, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                info!("Frame received - {}", helpers::format_uplink(v)?);
                mesh::handle_uplink(border_gateway, v).await?;
            }
        }
        Some(gw::event::Event::GatewayStats(_)) => {
            if border_gateway {
                proxy::send_event(event).await?;
            }
        }
        _ => {}
    }

    Ok(())
}

async fn handle_mesh_event_msg(border_gateway: bool, event: gw::Event) -> Result<()> {
    trace!("Handling mesh event, event: {:?}", event);

    if let Some(gw::event::Event::UplinkFrame(v)) = &event.event {
        if let Some(rx_info) = &v.rx_info {
            // Filter out frames with invalid CRC.
            if rx_info.crc_status() != gw::CrcStatus::CrcOk {
                debug!(
                    "Discarding uplink, CRC != OK, uplink_id: {}",
                    rx_info.uplink_id
                );
                return Ok(());
            }
        }

        // The mesh event msg must always be a proprietary payload.
        if v.phy_payload.first().cloned().unwrap_or_default() & 0xe0 == 0xe0 {
            mesh::handle_mesh(border_gateway, v).await?;
        }
    }

    Ok(())
}

async fn send_command(cmd: gw::Command) -> Result<Vec<u8>> {
    trace!("Sending command, command: {:?}", cmd,);

    let cmd_chan = CONCENTRATORD_CMD_CHAN
        .get()
        .ok_or_else(|| anyhow!("CONCENTRATORD_CMD_CHAN is not set"))?;

    let (cmd_tx, cmd_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_chan.send((cmd, cmd_tx))?;
    cmd_rx.await?
}

async fn send_mesh_command(cmd: gw::Command) -> Result<Vec<u8>> {
    trace!("Sending mesh command, command: {:?}", cmd);

    let cmd_chan = MESH_CONCENTRATORD_CMD_CHAN
        .get()
        .ok_or_else(|| anyhow!("MESH_CONCENTRATORD_CMD_CHAN is not set"))?;

    let (cmd_tx, cmd_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_chan.send((cmd, cmd_tx))?;
    cmd_rx.await?
}

pub async fn mesh(pl: gw::DownlinkFrame) -> Result<()> {
    info!("Sending mesh frame - {}", helpers::format_downlink(&pl)?);
    let downlink_id = pl.downlink_id;

    let tx_ack = {
        let pl = gw::Command {
            command: Some(gw::command::Command::SendDownlinkFrame(pl)),
        };
        let resp_b = send_mesh_command(pl).await?;
        gw::DownlinkTxAck::decode(resp_b.as_slice())?
    };
    helpers::tx_ack_to_err(&tx_ack)?;
    info!("Enqueue acknowledged, downlink_id: {}", downlink_id);
    Ok(())
}

pub async fn send_downlink(pl: gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    info!(
        "Sending downlink frame - {}",
        helpers::format_downlink(&pl)?
    );

    let pl = gw::Command {
        command: Some(gw::command::Command::SendDownlinkFrame(pl)),
    };
    let resp_b = send_command(pl).await?;
    let tx_ack = gw::DownlinkTxAck::decode(resp_b.as_slice())?;

    Ok(tx_ack)
}

pub async fn send_gateway_configuration(pl: gw::GatewayConfiguration) -> Result<()> {
    info!("Sending gateway configuration, version: {}", pl.version);

    let pl = gw::Command {
        command: Some(gw::command::Command::SetGatewayConfiguration(pl)),
    };
    let _ = send_command(pl).await?;

    Ok(())
}

pub async fn get_relay_id() -> Result<[u8; 4]> {
    trace!("Getting relay ID");

    Ok(*RELAY_ID
        .get()
        .ok_or_else(|| anyhow!("RELAY_ID is not set"))?
        .lock()
        .await)
}

pub async fn get_gateway_id() -> Result<[u8; 8]> {
    trace!("Getting gateway ID");

    Ok(*GATEWAY_ID
        .get()
        .ok_or_else(|| anyhow!("GATEWAY_ID is not set"))?
        .lock()
        .await)
}

fn send_zmq_command(sock: &mut zmq::Socket, cmd: &gw::Command) -> Result<Vec<u8>> {
    debug!("Sending command to socket, command: {:?}", &cmd,);
    sock.send(cmd.encode_to_vec(), 0)?;

    // set poller so that we can timeout after 100ms
    let mut items = [sock.as_poll_item(zmq::POLLIN)];
    zmq::poll(&mut items, 100)?;
    if !items[0].is_readable() {
        return Err(anyhow!("Could not read down response"));
    }

    // read tx ack response
    let resp_b: &[u8] = &sock.recv_bytes(0)?;
    Ok(resp_b.to_vec())
}

fn receive_zmq_event(sock: &mut zmq::Socket) -> Result<gw::Event> {
    let b = sock.recv_bytes(0)?;
    let event = gw::Event::decode(b.as_slice())?;
    Ok(event)
}
