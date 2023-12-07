mod balancer;
mod server;

use balancer::BalancerError;
use clap::Parser;
use std::fmt::Display;

#[derive(Parser, Debug)]
struct Params {}

impl Display for BalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Balancer Error: {}", self)
    }
}

async fn app(params: Params) -> Result<(), BalancerError> {
    let balancer = balancer::new();
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
