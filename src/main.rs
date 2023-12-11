mod api_cli;
mod api_service;
mod balancer;
mod balancer_service;
mod health_check_service;
mod server;

use anyhow::Result;
use api_cli::{add_server, delete_server, update_server, ServerConfigs, ServerURL};
use balancer::Balancer;
use clap::Parser;
use clap::{Args, Subcommand};
#[derive(Parser, Debug)]
struct BalancerParams {
    /// Config yaml file path for servers configurations
    #[arg(short, long)]
    config: Option<String>,

    /// Load balancer port. load balancer will delegate incoming requests on this port to suitable server
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// Load balancer api port. load balancer will accept requests to modify server configs on this port
    #[arg(short, long, default_value_t = 8000)]
    api_port: u16,

    #[arg(short, long, default_value_t = false)]
    debug: bool,
}

async fn app(params: &BalancerParams) -> Result<()> {
    if params.debug {
        simple_logger::SimpleLogger::new()
            .with_level(log::LevelFilter::Debug)
            .init()?;
    } else {
        simple_logger::SimpleLogger::new()
            .with_level(log::LevelFilter::Info)
            .init()?;
    }
    let mut balancer = Balancer::new(params.config.clone(), params.port, params.api_port)?;
    balancer.listen().await?;

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Listen(BalancerParams),
    AddServer(ServerConfigs),
    DeleteServer(ServerURL),
    UpdateServer(ServerConfigs),
}

#[derive(Args)]
struct AddArgs {
    name: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = match &cli.command {
        Commands::Listen(params) => app(params).await,
        Commands::AddServer(configs) => add_server(configs).await,
        Commands::DeleteServer(configs) => delete_server(configs).await,
        Commands::UpdateServer(configs) => update_server(configs).await,
    } {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}
