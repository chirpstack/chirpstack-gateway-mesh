use std::collections::HashMap;
use std::process::Stdio;
use std::time::SystemTime;

use anyhow::Result;
use log::error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, OnceCell};

use crate::{config::Configuration, packets};

static COMMANDS: OnceCell<HashMap<u8, Vec<String>>> = OnceCell::const_new();
static LAST_TIMESTAMP: OnceCell<Mutex<Option<SystemTime>>> = OnceCell::const_new();

pub async fn setup(conf: &Configuration) -> Result<()> {
    // Only Relay Gateways process commands.
    if conf.mesh.border_gateway {
        return Ok(());
    }

    // Set commands.
    COMMANDS
        .set(
            conf.commands
                .commands
                .iter()
                .map(|(k, v)| (k.parse().unwrap(), v.clone()))
                .collect(),
        )
        .map_err(|_| anyhow!("OnceCell set error"))?;

    Ok(())
}

pub async fn execute_commands(pl: &packets::CommandPayload) -> Result<Vec<packets::Event>> {
    // Validate that the command timestamp did increment, compared to previous
    // command payload.
    if let Some(ts) = get_last_timestamp().await {
        if ts >= pl.timestamp {
            return Err(anyhow!(
                "Command timestamp did not increment compared to previous command payload"
            ));
        }
    }

    // Store the command timestamp.
    set_last_timestamp(pl.timestamp).await;

    // Execute the commands and capture the response events.
    let mut out = vec![];
    for cmd in &pl.commands {
        let resp = match cmd {
            packets::Command::Proprietary((t, v)) => execute_proprietary(*t, v).await,
        };

        match resp {
            Ok(v) => out.push(v),
            Err(e) => error!("Execute command error: {}", e),
        }
    }

    Ok(out)
}

async fn execute_proprietary(typ: u8, value: &[u8]) -> Result<packets::Event> {
    let args = COMMANDS
        .get()
        .ok_or_else(|| anyhow!("COMMANDS is not set"))?
        .get(&typ)
        .ok_or_else(|| anyhow!("Command type {} is not configured", typ))?;

    if args.is_empty() {
        return Err(anyhow!("Command for command type {} is empty", typ,));
    }

    let mut cmd = Command::new(&args[0]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Add addition args.
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    // Spawn process
    let mut child = cmd.spawn()?;

    // Write stdin
    let mut stdin = child.stdin.take().unwrap();
    tokio::spawn({
        let b = value.to_vec();
        async move { stdin.write(&b).await }
    });

    // Wait for output
    let out = child.wait_with_output().await?;
    Ok(packets::Event::Proprietary((typ, out.stdout)))
}

async fn get_last_timestamp() -> Option<SystemTime> {
    LAST_TIMESTAMP
        .get_or_init(|| async { Mutex::new(None) })
        .await
        .lock()
        .await
        .clone()
}

async fn set_last_timestamp(ts: SystemTime) {
    let mut last_ts = LAST_TIMESTAMP
        .get_or_init(|| async { Mutex::new(None) })
        .await
        .lock()
        .await;

    *last_ts = Some(ts);
}
