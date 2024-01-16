use anyhow::Result;

use crate::config::Configuration;
use crate::{backend, proxy};

pub async fn run(conf: &Configuration) -> Result<()> {
    proxy::setup(conf)?;
    backend::setup(conf).await?;

    Ok(())
}
