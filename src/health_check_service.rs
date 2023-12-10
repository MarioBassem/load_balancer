use crossbeam_channel::{select, Receiver, Sender};
use reqwest::StatusCode;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::time::{sleep, Duration};

use crate::server::Server;

// used to start/stop health check workers
pub(crate) enum HealthCheckRequest {
    Stop(String),
    Start(String, u64),
}

// used to send health report to balancer
#[derive(Debug)]
pub(crate) enum HealthReport {
    Healthy(String),
    Unhealhty(String),
}

pub(crate) async fn health_check_service(
    servers: Vec<(String, u64)>,
    tx: Sender<HealthReport>,         // to send healthy/unhealthy servers
    rx: Receiver<HealthCheckRequest>, // to receive stop/add requests
) {
    log::debug!("health check service started...");
    let mut map = HashMap::new();
    for (url, period) in servers {
        let (mytx, myrx) = crossbeam_channel::bounded(0);
        map.insert(url.clone(), mytx);
        tokio::spawn(health_check(url, period, tx.clone(), myrx));
    }

    loop {
        let req = match rx.recv() {
            Ok(req) => req,
            Err(error) => {
                log::error!("health check service failed to receive signal: {}", error);
                continue;
            }
        };

        match req {
            HealthCheckRequest::Start(url, period) => {
                let (mytx, myrx) = crossbeam_channel::bounded(0);
                map.insert(url.clone(), mytx);
                tokio::spawn(health_check(url, period, tx.clone(), myrx));
            }
            HealthCheckRequest::Stop(url) => {
                let worker_sender = match map.remove(&url) {
                    Some(url) => url,
                    None => {
                        log::error!("failed to find server {}", url);
                        continue;
                    }
                };

                if let Err(error) = worker_sender.send(()) {
                    log::error!(
                        "failed to send stop signal to {} health check worker: {}",
                        url,
                        error
                    );
                }
            }
        }
    }
}

async fn health_check(url: String, period: u64, tx: Sender<HealthReport>, rx: Receiver<()>) {
    let handler = tokio::spawn(async move {
        loop {
            log::debug!("health check for url: {}", url);
            let req = match reqwest::get(url.clone()).await {
                Ok(req) => req,
                Err(error) => {
                    log::error!("failed to send health check request to {}: {}", url, error);
                    continue;
                }
            };

            let status = req.status().as_u16();
            if status < 200 || status >= 400 {
                log::debug!(
                    "server {} failed health check with status code {}",
                    url,
                    status
                );

                if let Err(error) = tx.send(HealthReport::Unhealhty(url.clone())) {
                    log::error!(
                        "failed to send {} health check failure signal to balancer: {}",
                        url,
                        error
                    );
                }
            } else {
                log::debug!("server {} is healthy", url);

                if let Err(error) = tx.send(HealthReport::Healthy(url.clone())) {
                    log::error!(
                        "failed to send {} health check failure signal to balancer: {}",
                        url,
                        error
                    );
                }
            }

            sleep(Duration::from_secs(period)).await;
        }
    });

    loop {
        log::debug!("waiting for health check kill sig");
        select! {
            recv(rx) -> receive_result =>{
                match receive_result{
                    Ok(()) => (),
                    Err(error) => {
                        log::error!("health check worker failed to receive signal: {}", error);
                        continue;
                    }
                };

                log::debug!("killing health check for ");
                handler.abort();
                return;
            }
        }
    }
}