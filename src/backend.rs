use std::time::Duration;

use anyhow::Result;
use chirpstack_api::prost::Message;
use log::{debug, error, info, trace};
use once_cell::sync::OnceCell;
use tokio::{
    sync::Mutex,
    time::{sleep, timeout},
};
use zeromq::{Socket, SocketRecv, SocketSend};

use crate::config::Configuration;
use crate::{helpers, proxy, relay};
use chirpstack_api::gw;

static CONCENTRATORD: OnceCell<Mutex<Backend>> = OnceCell::new();
static RELAY_CONCENTRATORD: OnceCell<Mutex<Backend>> = OnceCell::new();

pub async fn setup(conf: &Configuration) -> Result<()> {
    setup_concentratord(conf).await?;
    setup_relay_concentratord(conf).await?;
    Ok(())
}

async fn setup_concentratord(conf: &Configuration) -> Result<()> {
    info!(
        "Setting up Concentratord backend, event_url: {}, command_url: {}",
        conf.backend.concentratord.event_url, conf.backend.concentratord.command_url
    );

    let mut event_sock = zeromq::SubSocket::new();
    event_sock
        .connect(&conf.backend.concentratord.event_url)
        .await?;
    event_sock.subscribe("").await?;

    let mut cmd_sock = zeromq::ReqSocket::new();
    cmd_sock
        .connect(&conf.backend.concentratord.command_url)
        .await?;

    let mut b = Backend {
        cmd_sock,
        gateway_id: None,
    };
    b.read_gateway_id().await?;

    tokio::spawn({
        let border_gateway = conf.relay.border_gateway;
        let filters = lrwn_filters::Filters {
            dev_addr_prefixes: conf.relay.filters.dev_addr_prefixes.clone(),
            join_eui_prefixes: conf.relay.filters.join_eui_prefixes.clone(),
        };

        async move {
            event_loop(border_gateway, event_sock, filters).await;
            println!("BOOM");
        }
    });

    CONCENTRATORD
        .set(Mutex::new(b))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

async fn setup_relay_concentratord(conf: &Configuration) -> Result<()> {
    info!(
        "Setting up Relay Concentratord backend, event_url: {}, command_url: {}",
        conf.backend.relay_concentratord.event_url, conf.backend.relay_concentratord.command_url
    );

    let mut event_sock = zeromq::SubSocket::new();
    event_sock
        .connect(&conf.backend.relay_concentratord.event_url)
        .await?;
    event_sock.subscribe("").await?;

    let mut cmd_sock = zeromq::ReqSocket::new();
    cmd_sock
        .connect(&conf.backend.relay_concentratord.command_url)
        .await?;

    let mut b = Backend {
        cmd_sock,
        gateway_id: None,
    };
    b.read_gateway_id().await?;

    tokio::spawn({
        let border_gateway = conf.relay.border_gateway;

        async move {
            relay_event_loop(border_gateway, event_sock).await;
        }
    });

    RELAY_CONCENTRATORD
        .set(Mutex::new(b))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

struct Backend {
    cmd_sock: zeromq::ReqSocket,
    // cmd_sock: Arc<Mutex<zmq::Socket>>,
    gateway_id: Option<[u8; 8]>,
}

impl Backend {
    async fn read_gateway_id(&mut self) -> Result<()> {
        trace!("Reading gateway ID");

        self.cmd_sock
            .send(
                vec![bytes::Bytes::from("gateway_id"), bytes::Bytes::from("")]
                    .try_into()
                    .map_err(|e| anyhow!("To ZMQ message error: {}", e))?,
            )
            .await?;
        let resp = timeout(Duration::from_millis(100), self.cmd_sock.recv()).await??;

        let mut gateway_id: [u8; 8] = [0; 8];
        gateway_id.copy_from_slice(&resp.get(0).map(|v| v.to_vec()).unwrap_or_default());

        info!("Retrieved gateway ID: {}", hex::encode(gateway_id));

        self.gateway_id = Some(gateway_id);

        Ok(())
    }

    async fn send_command(&mut self, cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
        trace!("Sending command, cmd: {}, bytes: {}", cmd, hex::encode(b));

        self.cmd_sock
            .send(
                vec![
                    bytes::Bytes::from(cmd.to_string()),
                    bytes::Bytes::from(b.to_vec()),
                ]
                .try_into()
                .map_err(|e| anyhow!("To ZMQ message error: {}", e))?,
            )
            .await?;

        timeout(Duration::from_millis(100), self.cmd_sock.recv())
            .await?
            .map(|v| v.get(0).map(|v| v.to_vec()).unwrap_or_default())
            .map_err(|e| anyhow!("ZMQ error: {}", e))
    }
}

async fn event_loop(
    border_gateway: bool,
    mut event_sock: zeromq::SubSocket,
    filters: lrwn_filters::Filters,
) {
    trace!("Starting event loop");

    loop {
        trace!("Reading next event from zmq socket");
        let resp = match event_sock.recv().await {
            Ok(v) => v,
            Err(e) => {
                error!("Error receiving from zmq socket: {}", e);
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Err(e) = handle_event_msg(
            border_gateway,
            &resp.get(0).map(|v| v.to_vec()).unwrap_or_default(),
            &resp.get(1).map(|v| v.to_vec()).unwrap_or_default(),
            &filters,
        )
        .await
        {
            error!("Handle event error: {}", e);
            continue;
        }
    }
}

async fn relay_event_loop(border_gateway: bool, mut event_sock: zeromq::SubSocket) {
    trace!("Starting relay event loop");

    loop {
        trace!("Reading next relay event from zmq socket");
        let resp = match event_sock.recv().await {
            Ok(v) => v,
            Err(e) => {
                error!("Error receiving from zmq socket: {}", e);
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Err(e) = handle_relay_event_msg(
            border_gateway,
            &resp.get(0).map(|v| v.to_vec()).unwrap_or_default(),
            &resp.get(1).map(|v| v.to_vec()).unwrap_or_default(),
        )
        .await
        {
            error!("Handle relay event error: {}", e);
            continue;
        }
    }
}

async fn handle_event_msg(
    border_gateway: bool,
    event: &[u8],
    pl: &[u8],
    filters: &lrwn_filters::Filters,
) -> Result<()> {
    let event = String::from_utf8(event.to_vec())?;

    trace!(
        "Handling event, event: {}, data: {}",
        event,
        hex::encode(pl)
    );

    match event.as_str() {
        "up" => {
            let pl = gw::UplinkFrame::decode(pl)?;

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
                if pl.phy_payload.first().cloned().unwrap_or_default() & 0xe0 != 0 {
                    debug!(
                        "Discarding proprietary uplink, uplink_id: {}",
                        rx_info.uplink_id
                    );
                    return Ok(());
                }

                // Filter uplinks based on DevAddr and JoinEUI filters.
                if !lrwn_filters::matches(&pl.phy_payload, filters) {
                    debug!(
                        "Discarding uplink because of dev_addr and join_eui filters, uplink_id: {}",
                        rx_info.uplink_id
                    )
                }

                relay::handle_uplink(border_gateway, pl).await?;
            }
        }
        "stats" => {
            if border_gateway {
                let pl = gw::GatewayStats::decode(pl)?;
                proxy::send_stats(&pl).await?;
            }
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

async fn handle_relay_event_msg(border_gateway: bool, event: &[u8], pl: &[u8]) -> Result<()> {
    let event = String::from_utf8(event.to_vec())?;

    trace!(
        "Handling relay event, event: {}, data: {}",
        event,
        hex::encode(pl)
    );

    match event.as_str() {
        "up" => {
            let pl = gw::UplinkFrame::decode(pl)?;

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

            // The relay event msg must always be a proprietary payload.
            if pl.phy_payload.first().cloned().unwrap_or_default() & 0xe0 != 0 {
                relay::handle_relay(border_gateway, pl).await?;
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

    let backend = CONCENTRATORD
        .get()
        .ok_or_else(|| anyhow!("CONCENTRATORD is not set"))?;
    let mut backend = backend.lock().await;

    backend.send_command(cmd, b).await
}

async fn send_relay_command(cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
    trace!(
        "Sending relay command, command: {}, data: {}",
        cmd,
        hex::encode(b)
    );

    let backend = RELAY_CONCENTRATORD
        .get()
        .ok_or_else(|| anyhow!("RELAY_CONCENTRATORD is not set"))?;
    let mut backend = backend.lock().await;

    backend.send_command(cmd, b).await
}

pub async fn relay(pl: &gw::DownlinkFrame) -> Result<()> {
    info!("Sending relay payload, downlink_id: {}", pl.downlink_id);

    let tx_ack = {
        let b = pl.encode_to_vec();
        let resp_b = send_relay_command("down", &b).await?;
        gw::DownlinkTxAck::decode(resp_b.as_slice())?
    };
    helpers::tx_ack_to_err(&tx_ack)?;
    info!("Enqueue acknowledged, downlink_id: {}", pl.downlink_id);
    Ok(())
}

pub async fn send_downlink(pl: &gw::DownlinkFrame) -> Result<gw::DownlinkTxAck> {
    info!("Sending downlink, downlink_id: {}", pl.downlink_id);

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

    let backend = RELAY_CONCENTRATORD
        .get()
        .ok_or_else(|| anyhow!("RELAY_CONCENTRATORD is not set"))?;
    let backend = backend.lock().await;

    let mut relay_id: [u8; 4] = [0; 4];
    relay_id.copy_from_slice(&backend.gateway_id.unwrap_or_default()[4..]);
    Ok(relay_id)
}

pub async fn get_gateway_id() -> Result<[u8; 8]> {
    trace!("Getting gateway ID");

    let backend = CONCENTRATORD
        .get()
        .ok_or_else(|| anyhow!("CONCENTRATORD is not set"))?;
    let backend = backend.lock().await;

    let mut gateway_id: [u8; 8] = [0; 8];
    gateway_id.copy_from_slice(&backend.gateway_id.unwrap_or_default());
    Ok(gateway_id)
}
