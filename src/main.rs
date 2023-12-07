mod balancer;
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

    #[clap(short, long, default_value_t = false)]
    debug: bool,
}

impl Display for BalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Balancer Error: {}", self)
    }
}

async fn app(params: Params) -> Result<(), BalancerError> {
    let balancer = balancer::new(params.config, params.port)?;
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
