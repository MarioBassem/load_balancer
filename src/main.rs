mod api_service;
mod balancer;
mod balancer_service;
mod health_check_service;
mod server;

use balancer::BalancerError;
use clap::Parser;
use std::fmt::Display;

#[derive(Parser, Debug)]
struct Params {
    #[clap(short, long)]
    config: Option<String>,

    #[clap(short, long, default_value_t = 3000)]
    port: u16,

    #[clap(short, long, default_value_t = 8000)]
    api_port: u16,

    #[clap(short, long, default_value_t = false)]
    debug: bool,
}

impl Display for BalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BalancerError::ConfigError(s) => {
                write!(f, "Config Error: {}", s)?;
            }
            BalancerError::IO(s) => {
                write!(f, "IO Error: {}", s)?;
            }
            BalancerError::MyError(s) => {
                write!(f, "Balancer Error: {}", s)?;
            }
            BalancerError::ParseError(s) => {
                write!(f, "Parsing Error: {}", s)?;
            }
        }
        Ok(())
    }
}

async fn app(params: Params) -> Result<(), BalancerError> {
    let mut balancer = balancer::new(params.config, params.port, params.api_port)?;
    balancer.listen().await?;

    Ok(())
}

#[tokio::main]
async fn main() {
    simple_logger::SimpleLogger::new().init().unwrap();

    let args = Params::parse();
    if let Err(e) = app(args).await {
        eprintln!("{:#}", e);
        std::process::exit(1);
    }
}
