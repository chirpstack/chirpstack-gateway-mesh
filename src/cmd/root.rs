use anyhow::Result;
use futures::stream::StreamExt;
use signal_hook::consts::signal::*;
use signal_hook_tokio::Signals;

use crate::config::Configuration;
use crate::{backend, commands, events, proxy};

pub async fn run(conf: &Configuration) -> Result<()> {
    proxy::setup(conf).await?;
    backend::setup(conf).await?;
    events::setup(conf).await?;
    commands::setup(conf).await?;

    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();

    let _ = signals.next().await;
    handle.close();

    Ok(())
}
