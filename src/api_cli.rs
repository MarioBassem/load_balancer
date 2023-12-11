use anyhow::{bail, Result};
use clap::Parser;
use reqwest::StatusCode;

use crate::server::Server;

#[derive(Parser, Debug)]
pub(crate) struct ServerConfigs {
    /// Server url
    #[arg(long)]
    url: String,

    /// Server name
    #[arg(long)]
    name: String,

    /// Server weight, higher values result in more requests delegated to this server
    #[arg(long, default_value_t = 1)]
    weight: u32,

    /// Balancer will perform a health check on this server each specified period
    #[arg(long, default_value_t = 10)]
    health_check_period: u64, // in seconds

    /// Balancer api port
    #[arg(long, default_value_t = 8000)]
    api_port: u16,
}

/// performs an http request to add a new server to the balancer's monitored servers
pub(crate) async fn add_server(configs: &ServerConfigs) -> Result<()> {
    let server = Server {
        name: configs.name.clone(),
        health_check_period: configs.health_check_period,
        url: configs.url.clone(),
        weight: configs.weight,
        connections: 0,
        healthy: false,
    };

    let server_str = serde_json::to_string(&server)?;
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/add", configs.api_port))
        .body(server_str)
        .send()
        .await?;

    if response.status() != StatusCode::OK {
        bail!(response.text().await?);
    }

    Ok(())
}

/// performs an http request to update a monitored server's configs
pub(crate) async fn update_server(configs: &ServerConfigs) -> Result<()> {
    let server = Server {
        name: configs.name.clone(),
        health_check_period: configs.health_check_period,
        url: configs.url.clone(),
        weight: configs.weight,
        connections: 0,
        healthy: false,
    };

    let server_str = serde_json::to_string(&server)?;
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/update", configs.api_port))
        .body(server_str)
        .send()
        .await?;

    if response.status() != StatusCode::OK {
        bail!(response.text().await?);
    }

    Ok(())
}

#[derive(Parser, Debug)]
pub(crate) struct ServerURL {
    /// Server url
    #[arg(long)]
    url: String,

    /// Balancer api port
    #[arg(long, default_value_t = 8000)]
    api_port: u16,
}

/// performs an http request to delete a monitored server
pub(crate) async fn delete_server(configs: &ServerURL) -> Result<()> {
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/delete", configs.api_port))
        .body(configs.url.clone())
        .send()
        .await?;

    if response.status() != StatusCode::OK {
        bail!(response.text().await?);
    }

    Ok(())
}
