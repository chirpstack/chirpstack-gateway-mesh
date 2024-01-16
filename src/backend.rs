use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;
use chirpstack_api::prost::Message;
use log::{debug, error, info, trace, warn};
use once_cell::sync::OnceCell;
use tokio::task;

use crate::config::Configuration;
use crate::{relay, ZMQ_CONTEXT};
use chirpstack_api::gw;

static CONCENTRATORD: OnceCell<Backend> = OnceCell::new();
static RELAY_CONCENTRATORD: OnceCell<Backend> = OnceCell::new();

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

    let zmq_ctx = ZMQ_CONTEXT.lock().unwrap();
    let event_sock = zmq_ctx.socket(zmq::SUB)?;
    event_sock.connect(&conf.backend.concentratord.event_url)?;
    event_sock.set_subscribe("".as_bytes())?;

    let cmd_sock = zmq_ctx.socket(zmq::REQ)?;
    cmd_sock.connect(&conf.backend.concentratord.command_url)?;

    let mut b = Backend {
        cmd_url: conf.backend.concentratord.command_url.clone(),
        cmd_sock: Mutex::new(cmd_sock),
        gateway_id: None,
    };
    b.read_gateway_id()?;

    tokio::spawn({
        let border_gateway = conf.relay.border_gateway;
        let filters = lrwn_filters::Filters {
            dev_addr_prefixes: conf.relay.filters.dev_addr_prefixes.clone(),
            join_eui_prefixes: conf.relay.filters.join_eui_prefixes.clone(),
        };

        async move {
            event_loop(border_gateway, event_sock, filters).await;
        }
    });

    CONCENTRATORD
        .set(b)
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

async fn setup_relay_concentratord(conf: &Configuration) -> Result<()> {
    info!(
        "Setting up Relay Concentratord backend, event_url: {}, command_url: {}",
        conf.backend.relay_concentratord.event_url, conf.backend.relay_concentratord.command_url
    );

    let zmq_ctx = ZMQ_CONTEXT.lock().unwrap();
    let event_sock = zmq_ctx.socket(zmq::SUB)?;
    event_sock.connect(&conf.backend.relay_concentratord.event_url)?;
    event_sock.set_subscribe("".as_bytes())?;

    let cmd_sock = zmq_ctx.socket(zmq::REQ)?;
    cmd_sock.connect(&conf.backend.relay_concentratord.command_url)?;

    let mut b = Backend {
        cmd_url: conf.backend.concentratord.command_url.clone(),
        cmd_sock: Mutex::new(cmd_sock),
        gateway_id: None,
    };
    b.read_gateway_id()?;

    tokio::spawn({
        let border_gateway = conf.relay.border_gateway;

        async move {
            relay_event_loop(border_gateway, event_sock).await;
        }
    });

    RELAY_CONCENTRATORD
        .set(b)
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

struct Backend {
    cmd_url: String,
    cmd_sock: Mutex<zmq::Socket>,
    gateway_id: Option<[u8; 8]>,
}

impl Backend {
    fn read_gateway_id(&mut self) -> Result<()> {
        let cmd_sock = self.cmd_sock.lock().unwrap();

        // send 'gateway_id' command with empty payload.
        cmd_sock.send("gateway_id", zmq::SNDMORE)?;
        cmd_sock.send("", 0)?;

        // set poller so that we can timeout after 100ms
        let mut items = [cmd_sock.as_poll_item(zmq::POLLIN)];
        zmq::poll(&mut items, 100)?;
        if !items[0].is_readable() {
            return Err(anyhow!("Could not read gateway id"));
        }

        let mut gateway_id: [u8; 8] = [0; 8];
        gateway_id.copy_from_slice(&cmd_sock.recv_bytes(0)?);
        self.gateway_id = Some(gateway_id);

        Ok(())
    }

    fn send_command(&self, cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
        let res = || -> Result<Vec<u8>> {
            let cmd_sock = self.cmd_sock.lock().unwrap();
            cmd_sock.send(cmd, zmq::SNDMORE)?;
            cmd_sock.send(b, 0)?;

            // set poller so that we can timeout after 100ms
            let mut items = [cmd_sock.as_poll_item(zmq::POLLIN)];
            zmq::poll(&mut items, 100)?;
            if !items[0].is_readable() {
                return Err(anyhow!("Could not read down response"));
            }

            // red tx ack response
            let resp_b: &[u8] = &cmd_sock.recv_bytes(0)?;
            Ok(resp_b.to_vec())
        }();

        if res.is_err() {
            loop {
                // Reconnect the CMD socket in case we received an error.
                // In case there was an issue with receiving data from the socket, it could mean
                // it is in a 'dirty' state. E.g. due to the error we did not read the full
                // response.
                if let Err(e) = self.reconnect_cmd_sock() {
                    error!(
                        "Re-connecting to Concentratord command API error, error: {}",
                        e
                    );
                    sleep(Duration::from_secs(1));
                    continue;
                }

                break;
            }
        }

        res
    }

    fn reconnect_cmd_sock(&self) -> Result<()> {
        warn!(
            "Re-connecting to Concentratord command API, command_url: {}",
            self.cmd_url
        );
        let zmq_ctx = ZMQ_CONTEXT.lock().unwrap();
        let mut cmd_sock = self.cmd_sock.lock().unwrap();
        *cmd_sock = zmq_ctx.socket(zmq::REQ)?;
        cmd_sock.connect(&self.cmd_url)?;
        Ok(())
    }
}

async fn event_loop(border_gateway: bool, event_sock: zmq::Socket, filters: lrwn_filters::Filters) {
    trace!("Starting event loop");
    let event_sock = Arc::new(Mutex::new(event_sock));

    loop {
        let event = match read_event(event_sock.clone()).await {
            Ok(v) => v,
            Err(err) => {
                error!("Receive event error, error: {}", err);
                continue;
            }
        };

        if event.len() != 2 {
            continue;
        }

        if let Err(err) = handle_event_msg(border_gateway, &event[0], &event[1], &filters).await {
            error!("Handle event error: {}", err);
            continue;
        }
    }
}

async fn relay_event_loop(border_gateway: bool, event_sock: zmq::Socket) {
    trace!("Starting relay event loop");
    let event_sock = Arc::new(Mutex::new(event_sock));

    loop {
        let event = match read_event(event_sock.clone()).await {
            Ok(v) => v,
            Err(err) => {
                error!("Receive event error, error: {}", err);
                continue;
            }
        };

        if event.len() != 2 {
            continue;
        }

        if let Err(err) = handle_relay_event_msg(border_gateway, &event[0], &event[1]).await {
            error!("Handle relay event error: {}", err);
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

                // Note that proprietary frames will always pass as these can't be
                // filtered.
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
            let pl = gw::GatewayStats::decode(pl)?;
            relay::handle_stats(border_gateway, pl).await?;
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

async fn handle_relay_event_msg(border_gateway: bool, event: &[u8], pl: &[u8]) -> Result<()> {
    let event = String::from_utf8(event.to_vec())?;

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
                relay::handle_uplink(border_gateway, pl).await?;
            }
        }
        _ => {
            return Ok(());
        }
    }

    Ok(())
}

async fn read_event(event_sock: Arc<Mutex<zmq::Socket>>) -> Result<Vec<Vec<u8>>> {
    task::spawn_blocking({
        move || -> Result<Vec<Vec<u8>>> {
            let event_sock = event_sock.lock().unwrap();

            // set poller so that we can timeout after 100ms
            let mut items = [event_sock.as_poll_item(zmq::POLLIN)];
            zmq::poll(&mut items, 100)?;
            if !items[0].is_readable() {
                return Ok(vec![]);
            }

            let msg = event_sock.recv_multipart(0)?;
            if msg.len() != 2 {
                return Err(anyhow!("Event must have two frames"));
            }
            Ok(msg)
        }
    })
    .await?
}

async fn send_command(cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
    task::spawn_blocking({
        let cmd = cmd.to_string();
        let b = b.to_vec();

        move || -> Result<Vec<u8>> {
            if let Some(backend) = CONCENTRATORD.get() {
                return backend.send_command(&cmd, &b);
            }

            Err(anyhow!("CONCENTRATORD is not set"))
        }
    })
    .await?
}

async fn send_relay_command(cmd: &str, b: &[u8]) -> Result<Vec<u8>> {
    task::spawn_blocking({
        let cmd = cmd.to_string();
        let b = b.to_vec();

        move || -> Result<Vec<u8>> {
            if let Some(backend) = RELAY_CONCENTRATORD.get() {
                return backend.send_command(&cmd, &b);
            }

            Err(anyhow!("RELAY_CONCENTRATORD is not set"))
        }
    })
    .await?
}

pub async fn relay(pl: &gw::DownlinkFrame) -> Result<()> {
    let tx_ack = {
        let b = pl.encode_to_vec();
        let resp_b = send_relay_command("down", &b).await?;
        gw::DownlinkTxAck::decode(resp_b.as_slice())?
    };

    let tx_ack_ok: Vec<gw::DownlinkTxAckItem> = tx_ack
        .items
        .iter()
        .filter(|v| v.status() == gw::TxAckStatus::Ok)
        .cloned()
        .collect();

    if !tx_ack_ok.is_empty() {
        return Ok(());
    }

    Err(anyhow!(
        "Relay failed: {}",
        tx_ack
            .items
            .last()
            .cloned()
            .unwrap_or_default()
            .status()
            .as_str_name()
    ))
}

pub async fn send_downlink(pl: &gw::DownlinkFrame) -> Result<()> {
    let tx_ack = {
        let b = pl.encode_to_vec();
        let resp_b = send_command("down", &b).await?;
        gw::DownlinkTxAck::decode(resp_b.as_slice())?
    };

    let tx_ack_ok: Vec<gw::DownlinkTxAckItem> = tx_ack
        .items
        .iter()
        .filter(|v| v.status() == gw::TxAckStatus::Ok)
        .cloned()
        .collect();

    if !tx_ack_ok.is_empty() {
        return Ok(());
    }

    Err(anyhow!(
        "Send downlink failed: {}",
        tx_ack
            .items
            .last()
            .cloned()
            .unwrap_or_default()
            .status()
            .as_str_name()
    ))
}

pub fn get_relay_id() -> Result<[u8; 4]> {
    if let Some(rc) = RELAY_CONCENTRATORD.get() {
        let mut relay_id: [u8; 4] = [0; 4];
        relay_id.copy_from_slice(&rc.gateway_id.unwrap_or_default()[4..])
    }

    Err(anyhow!("RELAY_CONCENTRATORD is not (yet) initialized"))
}
