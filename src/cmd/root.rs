use anyhow::Result;

use crate::backend;
use crate::config::Configuration;

pub async fn run(conf: &Configuration) -> Result<()> {
    backend::setup(conf).await?;

    Ok(())
}
