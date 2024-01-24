use std::thread::sleep;
use std::time::Duration;
use std::{process, str::FromStr};

use clap::{Parser, Subcommand};
use log::info;

use chirpstack_gateway_relay::{cmd, config, logging};

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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    config::Configuration::load(&cli.config).expect("Read configuration error");

    if let Some(Commands::Configfile {}) = &cli.command {
        cmd::configfile::run();
        process::exit(0);
    }

    let conf = config::get();
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

    info!(
        "Starting {} (border_gateway: {},version: {}, docs: {})",
        env!("CARGO_PKG_DESCRIPTION"),
        conf.relay.border_gateway,
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_HOMEPAGE"),
    );

    cmd::root::run(&conf).await.unwrap();
}
