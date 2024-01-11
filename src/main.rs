#[macro_use]
extern crate anyhow;

use std::thread::sleep;
use std::time::Duration;
use std::{process, str::FromStr};

use clap::{Parser, Subcommand};
use log::info;

mod backend;
mod cmd;
mod config;
mod logging;
mod packets;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Vec<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the configuration template
    Configfile {},
    /// Operate the Relay as Border Gateway
    BorderGateway {},
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let conf = config::Configuration::get(&cli.config).expect("Read configuration error");

    if let Some(Commands::Configfile {}) = &cli.command {
        cmd::configfile::run(&conf);
        process::exit(0);
    }

    let log_level = log::Level::from_str(&conf.logging.level).expect("Parse log_level error");

    // Loop until success, as this will fail when syslog hasn't been fully started.
    while let Err(e) = logging::setup(
        env!("CARGO_PKG_NAME"),
        log_level,
        conf.logging.log_to_syslog,
    ) {
        println!("Setup log error: {}", e);
        sleep(Duration::from_secs(1))
    }

    let border_gateway = if let Some(Commands::BorderGateway {}) = &cli.command {
        true
    } else {
        false
    };

    info!(
        "Starting {} (border_gateway: {},version: {}, docs: {})",
        env!("CARGO_PKG_DESCRIPTION"),
        border_gateway,
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_HOMEPAGE"),
    );

    if border_gateway {
        cmd::border_gateway::run(&conf).await.unwrap();
    } else {
        cmd::root::run(&conf).await.unwrap();
    }
}
