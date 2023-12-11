use async_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};

// used to start/stop health check workers
#[derive(Debug)]
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
    log::debug!("service started...");
    let mut map = HashMap::new();
    for (url, period) in servers {
        let (mytx, myrx) = bounded(1);
        map.insert(url.clone(), mytx);
        tokio::spawn(health_check(url, period, tx.clone(), myrx));
    }

    loop {
        let req = match rx.recv().await {
            Ok(req) => req,
            Err(error) => {
                log::error!("failed to receive signal: {}", error);
                continue;
            }
        };

        match req {
            HealthCheckRequest::Start(url, period) => {
                let (mytx, myrx) = bounded(1);
                map.insert(url.clone(), mytx);
                let _ = tokio::spawn(health_check(url, period, tx.clone(), myrx)).await;
            }
            HealthCheckRequest::Stop(url) => {
                let worker_sender = match map.remove(&url) {
                    Some(url) => url,
                    None => {
                        log::error!("failed to find server {}", url);
                        continue;
                    }
                };

                if let Err(error) = worker_sender.send(()).await {
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
    let url_clone = url.clone();
    let handler = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(period)).await;

            log::debug!("performing health check for url: {}", url);
            let req = match reqwest::get(url.clone()).await {
                Ok(req) => req,
                Err(error) => {
                    log::error!("failed to send health check request to {}: {}", url, error);
                    continue;
                }
            };

            let status = req.status().as_u16();
            if !(200..400).contains(&status) {
                if let Err(error) = tx.send(HealthReport::Unhealhty(url.clone())).await {
                    log::error!(
                        "failed to send {} health check failure signal to balancer: {}",
                        url,
                        error
                    );
                }
            } else {
                if let Err(error) = tx.send(HealthReport::Healthy(url.clone())).await {
                    log::error!(
                        "failed to send {} health check failure signal to balancer: {}",
                        url,
                        error
                    );
                }
            }
        }
    });

    loop {
        match rx.recv().await {
            Ok(()) => (),
            Err(error) => {
                log::error!("health check worker failed to receive signal: {}", error);
                continue;
            }
        };

        log::debug!("killing health check service for {}", url_clone);
        handler.abort();
        return;
    }
}
