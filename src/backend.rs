use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::{error, info, trace};
use once_cell::sync::OnceCell;
use tokio::task;

use crate::config::Configuration;

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

    let zmq_ctx = zmq::Context::new();
    let event_sock = zmq_ctx.socket(zmq::SUB)?;
    event_sock.connect(&conf.backend.concentratord.event_url)?;
    event_sock.set_subscribe("".as_bytes())?;

    let cmd_sock = zmq_ctx.socket(zmq::REQ)?;
    cmd_sock.connect(&conf.backend.concentratord.command_url)?;

    let mut b = Backend {
        ctx: zmq_ctx,
        cmd_url: conf.backend.concentratord.command_url.clone(),
        cmd_sock: Mutex::new(cmd_sock),
        gateway_id: None,
    };
    b.read_gateway_id()?;

    tokio::spawn({
        let filters = lrwn_filters::Filters {
            dev_addr_prefixes: conf.relay.filters.dev_addr_prefixes.clone(),
            join_eui_prefixes: conf.relay.filters.join_eui_prefixes.clone(),
        };

        async move {
            event_loop(event_sock, filters).await;
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

    let zmq_ctx = zmq::Context::new();
    let event_sock = zmq_ctx.socket(zmq::SUB)?;
    event_sock.connect(&conf.backend.relay_concentratord.event_url)?;
    event_sock.set_subscribe("".as_bytes())?;

    let cmd_sock = zmq_ctx.socket(zmq::REQ)?;
    cmd_sock.connect(&conf.backend.relay_concentratord.command_url)?;

    let mut b = Backend {
        ctx: zmq_ctx,
        cmd_url: conf.backend.concentratord.command_url.clone(),
        cmd_sock: Mutex::new(cmd_sock),
        gateway_id: None,
    };
    b.read_gateway_id()?;

    tokio::spawn(async move {
        relay_event_loop(event_sock).await;
    });

    RELAY_CONCENTRATORD
        .set(b)
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

struct Backend {
    ctx: zmq::Context,
    cmd_url: String,
    cmd_sock: Mutex<zmq::Socket>,
    gateway_id: Option<String>,
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
        let gateway_id = cmd_sock.recv_bytes(0)?;
        self.gateway_id = Some(hex::encode(gateway_id));

        Ok(())
    }
}

async fn event_loop(event_sock: zmq::Socket, filters: lrwn_filters::Filters) {
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
    }
}

async fn relay_event_loop(event_sock: zmq::Socket) {
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
    }
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
