use anyhow::Result;
use chirpstack_api::gw;
use chirpstack_api::prost::Message;
use log::{error, info, trace};
use once_cell::sync::OnceCell;
use tokio::sync::Mutex;
use zeromq::{Socket, SocketRecv, SocketSend};

use crate::backend;
use crate::cleanup_socket_file;
use crate::config::Configuration;
use crate::relay;

static EVENT_SOCKET: OnceCell<Mutex<zeromq::PubSocket>> = OnceCell::new();

pub enum Command {
    Timeout,
    Unknown(String, Vec<u8>),
    Downlink(gw::DownlinkFrame),
    GatewayID,
    Configuration(gw::GatewayConfiguration),
}

pub async fn setup(conf: &Configuration) -> Result<()> {
    if !conf.relay.border_gateway {
        return Ok(());
    }

    info!(
        "Setting up Concentratord proxy API, event_bind: {}, command_bind: {}",
        conf.relay.proxy_api.event_bind, conf.relay.proxy_api.command_bind
    );

    let mut event_sock = zeromq::PubSocket::new();
    cleanup_socket_file(&conf.relay.proxy_api.event_bind).await;
    event_sock.bind(&conf.relay.proxy_api.event_bind).await?;

    EVENT_SOCKET
        .set(Mutex::new(event_sock))
        .map_err(|_| anyhow!("OnceCell set error"))?;

    let mut cmd_sock = zeromq::RepSocket::new();
    cleanup_socket_file(&conf.relay.proxy_api.command_bind).await;
    cmd_sock.bind(&conf.relay.proxy_api.command_bind).await?;

    tokio::spawn({
        async move {
            command_loop(cmd_sock).await;
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

    let b = pl.encode_to_vec();
    let event_socket = EVENT_SOCKET
        .get()
        .ok_or_else(|| anyhow!("EVENT_SOCKET is not set"))?;
    let mut event_socket = event_socket.lock().await;

    event_socket
        .send(
            vec![bytes::Bytes::from("up"), bytes::Bytes::from(b)]
                .try_into()
                .map_err(|e| anyhow!("To ZMQ message error: {}", e))?,
        )
        .await?;

    Ok(())
}

pub async fn send_stats(pl: &gw::GatewayStats) -> Result<()> {
    info!("Sending stats event");

    let b = pl.encode_to_vec();
    let event_socket = EVENT_SOCKET
        .get()
        .ok_or_else(|| anyhow!("EVENT_SOCKET is not set"))?;
    let mut event_socket = event_socket.lock().await;

    event_socket
        .send(
            vec![bytes::Bytes::from("stats"), bytes::Bytes::from(b)]
                .try_into()
                .map_err(|e| anyhow!("To ZMQ message error: {}", e))?,
        )
        .await?;

    Ok(())
}

async fn command_loop(mut rep_sock: zeromq::RepSocket) {
    trace!("Starting command loop");

    loop {
        trace!("Reading next command from zmq socket");
        let resp = match rep_sock.recv().await {
            Ok(v) => v,
            Err(e) => {
                error!("Error receiving from zmq socket: {}", e);
                continue;
            }
        };

        let cmd = match String::from_utf8(resp.get(0).map(|v| v.to_vec()).unwrap_or_default()) {
            Ok(v) => v,
            Err(e) => {
                error!("Error parsing command: {}", e);
                continue;
            }
        };
        let pl = resp.get(1).cloned().unwrap_or_default();

        let resp = match handle_command(&cmd, pl).await {
            Ok(v) => v,
            Err(e) => {
                error!("Handle command error: {}", e);
                Vec::new()
            }
        };

        if let Err(e) = rep_sock.send(resp.into()).await {
            error!("Error sending response to zmq socket: {}", e);
        }
    }
}

async fn handle_command(cmd: &str, pl: bytes::Bytes) -> Result<Vec<u8>> {
    Ok(match cmd {
        "config" => {
            let pl = gw::GatewayConfiguration::decode(pl)?;
            info!("Configuration command received, version: {}", pl.version);
            backend::send_gateway_configuration(&pl).await?;
            Vec::new()
        }
        "down" => {
            let pl = gw::DownlinkFrame::decode(pl)?;
            info!("Downlink command received, downlink_id: {}", pl.downlink_id);
            relay::handle_downlink(pl)
                .await
                .map(|v| v.encode_to_vec())?
        }
        "gateway_id" => {
            info!("Get gateway id command received");
            backend::get_gateway_id().await.map(|v| v.to_vec())?
        }
        _ => {
            return Err(anyhow!("Unexpected command: {}", cmd));
        }
    })
}
