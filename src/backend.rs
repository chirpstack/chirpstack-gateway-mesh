use std::sync::OnceLock;
use std::thread;

use anyhow::Result;
use chirpstack_api::prost::Message;
use log::{debug, error, info, trace};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::config::Configuration;
use crate::{helpers, mesh, proxy};
use chirpstack_api::gw;

static GATEWAY_ID: OnceLock<Mutex<[u8; 8]>> = OnceLock::new();
static RELAY_ID: OnceLock<Mutex<[u8; 4]>> = OnceLock::new();

static CONCENTRATORD_CMD_CHAN: OnceLock<CommandChannel> = OnceLock::new();
static MESH_CONCENTRATORD_CMD_CHAN: OnceLock<CommandChannel> = OnceLock::new();

type Event = (String, Vec<u8>);
type Command = ((String, Vec<u8>), oneshot::Sender<Result<Vec<u8>>>);
type CommandChannel = mpsc::UnboundedSender<Command>;

pub async fn setup(conf: &Configuration) -> Result<()> {
    setup_concentratord(conf).await?;
    setup_mesh_conncentratord(conf).await?;
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
                let resp = send_zmq_command(&mut sock, &cmd);
                cmd.1.send(resp).unwrap();
            }

            error!("Concentratord command loop has been interrupted");
        }
    });

    // Read Gateway ID.

    trace!("Reading Gateway ID");
    let mut gateway_id: [u8; 8] = [0; 8];
    let (gateway_id_tx, gateway_id_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_tx.send((("gateway_id".to_string(), vec![]), gateway_id_tx))?;
    let resp = gateway_id_rx.await??;
    gateway_id.copy_from_slice(&resp);
    info!("Retrieved Gateway ID: {}", hex::encode(gateway_id));
    GATEWAY_ID
        .set(Mutex::new(gateway_id))
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Set CMD channel.

    CONCENTRATORD_CMD_CHAN
        .set(cmd_tx)
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Setup ZMQ event.

    let (event_tx, event_rx) = mpsc::unbounded_channel::<Event>();

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

async fn setup_mesh_conncentratord(conf: &Configuration) -> Result<()> {
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
                let resp = send_zmq_command(&mut sock, &cmd);
                cmd.1.send(resp).unwrap();
            }

            error!("Mesh Concentratord command loop has been interrupted");
        }
    });

    // Read Relay ID.
    trace!("Reading Gateway ID");

    let (gateway_id_tx, gateway_id_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_tx.send((("gateway_id".to_string(), vec![]), gateway_id_tx))?;
    let resp = gateway_id_rx.await??;
    info!("Retrieved Gateway ID: {}", hex::encode(&resp));

    let mut relay_id: [u8; 4] = [0; 4];
    relay_id.copy_from_slice(&resp[4..]);
    RELAY_ID
        .set(Mutex::new(relay_id))
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // set CMD channel.

    MESH_CONCENTRATORD_CMD_CHAN
        .set(cmd_tx)
        .map_err(|e| anyhow!("OnceLock error: {:?}", e))?;

    // Setup ZMQ event.

    let (event_tx, event_rx) = mpsc::unbounded_channel::<Event>();

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
    mut event_rx: mpsc::UnboundedReceiver<Event>,
    filters: lrwn_filters::Filters,
) {
    trace!("Starting event loop");
    while let Some(event) = event_rx.recv().await {
        if let Err(e) = handle_event_msg(
            border_gateway,
            border_gateway_ignore_direct_uplinks,
            &event,
            &filters,
        )
        .await
        {
            error!("Handle event error: {}", e);
            continue;
        }
    }
}

async fn mesh_event_loop(border_gateway: bool, mut event_rx: mpsc::UnboundedReceiver<Event>) {
    trace!("Starting mesh event loop");
    while let Some(event) = event_rx.recv().await {
        if let Err(e) = handle_mesh_event_msg(border_gateway, &event).await {
            error!("Handle mesh event error: {}", e);
            continue;
        }
    }
}

async fn handle_event_msg(
    border_gateway: bool,
    border_gateway_ignore_direct_uplinks: bool,
    event: &Event,
    filters: &lrwn_filters::Filters,
) -> Result<()> {
    trace!(
        "Handling event, event: {}, data: {}",
        event.0,
        hex::encode(&event.1)
    );

    match event.0.as_str() {
        "up" => {
            let pl = gw::UplinkFrame::decode(event.1.as_slice())?;

            if let Some(rx_info) = &pl.rx_info {
                // Filter out frames with invalid CRC.
                if rx_info.crc_status() != gw::CrcStatus::CrcOk {
                    debug!(
                        "Discarding uplink, CRC != OK, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Filter out proprietary payloads.
                if pl.phy_payload.first().cloned().unwrap_or_default() & 0xe0 == 0xe0 {
                    debug!(
                        "Discarding proprietary uplink, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Ignore direct uplinks.
                if border_gateway_ignore_direct_uplinks {
                    debug!("Discarding direct uplink because of border_gateway_ignore_direct_uplinks setting, uplink_id: {}", rx_info.uplink_id);
                    return Ok(());
                }

                // Filter uplinks based on DevAddr and JoinEUI filters.
                if !lrwn_filters::matches(&pl.phy_payload, filters) {
                    debug!(
                        "Discarding uplink because of dev_addr and join_eui filters, uplink_id: {}",
                        rx_info.uplink_id
                    )
                }

                info!("Frame received - {}", helpers::format_uplink(&pl)?);
                mesh::handle_uplink(border_gateway, pl).await?;
            }
        }
        "stats" => {
            if border_gateway {
                let pl = gw::GatewayStats::decode(event.1.as_slice())?;
                info!("Gateway stats received, gateway_id: {}", pl.gateway_id);
                proxy::send_stats(&pl).await?;
            }
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

async fn handle_mesh_event_msg(border_gateway: bool, event: &Event) -> Result<()> {
    trace!(
        "Handling mesh event, event: {}, data: {}",
        event.0,
        hex::encode(&event.1)
    );

    match event.0.as_str() {
        "up" => {
            let pl = gw::UplinkFrame::decode(event.1.as_slice())?;

            if let Some(rx_info) = &pl.rx_info {
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
            if pl.phy_payload.first().cloned().unwrap_or_default() & 0xe0 == 0xe0 {
                info!("Mesh frame received - {}", helpers::format_uplink(&pl)?);
                mesh::handle_mesh(border_gateway, pl).await?;
            }
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

async fn send_command(cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
    trace!(
        "Sending command, command: {}, data: {}",
        cmd,
        hex::encode(b)
    );

    let cmd_chan = CONCENTRATORD_CMD_CHAN
        .get()
        .ok_or_else(|| anyhow!("CONCENTRATORD_CMD_CHAN is not set"))?;

    let (cmd_tx, cmd_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_chan.send(((cmd.to_string(), b.to_vec()), cmd_tx))?;
    cmd_rx.await?
}

async fn send_mesh_command(cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
    trace!(
        "Sending mesh command, command: {}, data: {}",
        cmd,
        hex::encode(b)
    );

    let cmd_chan = MESH_CONCENTRATORD_CMD_CHAN
        .get()
        .ok_or_else(|| anyhow!("MESH_CONCENTRATORD_CMD_CHAN is not set"))?;

    let (cmd_tx, cmd_rx) = oneshot::channel::<Result<Vec<u8>>>();
    cmd_chan.send(((cmd.to_string(), b.to_vec()), cmd_tx))?;
    cmd_rx.await?
}

pub async fn mesh(pl: &gw::DownlinkFrame) -> Result<()> {
    info!("Sending mesh frame - {}", helpers::format_downlink(pl)?);

    let tx_ack = {
        let b = pl.encode_to_vec();
        let resp_b = send_mesh_command("down", &b).await?;
        gw::DownlinkTxAck::decode(resp_b.as_slice())?
    };
    helpers::tx_ack_to_err(&tx_ack)?;
    info!("Enqueue acknowledged, downlink_id: {}", pl.downlink_id);
    Ok(())
}

pub async fn send_downlink(pl: &gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    info!("Sending downlink frame - {}", helpers::format_downlink(pl)?);

    let b = pl.encode_to_vec();
    let resp_b = send_command("down", &b).await?;
    let tx_ack = gw::DownlinkTxAck::decode(resp_b.as_slice())?;

    Ok(tx_ack)
}

pub async fn send_gateway_configuration(pl: &gw::GatewayConfiguration) -> Result<()> {
    info!("Sending gateway configuration, version: {}", pl.version);

    let b = pl.encode_to_vec();
    let _ = send_command("config", &b).await?;

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

fn send_zmq_command(sock: &mut zmq::Socket, cmd: &Command) -> Result<Vec<u8>> {
    debug!(
        "Sending command to socket, command: {}, payload: {}",
        &cmd.0 .0,
        hex::encode(&cmd.0 .1)
    );

    sock.send(&cmd.0 .0, zmq::SNDMORE)?;
    sock.send(&cmd.0 .1, 0)?;

    // set poller so that we can timeout after 100ms
    let mut items = [sock.as_poll_item(zmq::POLLIN)];
    zmq::poll(&mut items, 100)?;
    if !items[0].is_readable() {
        return Err(anyhow!("Could not read down response"));
    }

    // red tx ack response
    let resp_b: &[u8] = &sock.recv_bytes(0)?;
    Ok(resp_b.to_vec())
}

fn receive_zmq_event(sock: &mut zmq::Socket) -> Result<Event> {
    let msg = sock.recv_multipart(0)?;
    if msg.len() != 2 {
        return Err(anyhow!("Event must have 2 frames"));
    }

    let event = String::from_utf8(msg[0].to_vec())?;
    let b = msg[1].to_vec();

    Ok((event, b))
}
