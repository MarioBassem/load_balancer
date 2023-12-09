use hyper_util::rt::TokioIo;
use serde_json;
use std::{future::Future, pin::Pin};
use tokio::net::{TcpListener, TcpStream};

use crossbeam_channel::{Receiver, Sender};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Incoming},
    service::Service,
    Request, Response,
};

use crate::balancer::{BalancerError, BalancerRequest, BalancerResponse};

pub(crate) async fn balancer_listener(
    listener: TcpListener,
    tx: Sender<BalancerRequest>,
    rx: Receiver<String>,
) {
    loop {
        let tx = tx.clone();
        let rx = rx.clone();
        let (stream, _) = match listener.accept().await {
            Ok(stream) => stream,
            Err(error) => {
                log::error!("failed to accept connection: {}", error);
                continue;
            }
        };

        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            // wait for balancer to send url
            let url: String = match rx.recv() {
                Ok(s) => s,
                Err(error) => {
                    log::error!("failed to receive server url from balancer: {}", error);
                    return;
                }
            };
            log::debug!("delegating to {}", url);
            let address = url.clone();
            let service = service_fn(move |req| delegate(url.clone(), req));

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                log::error!("Failed to serve connection: {:?}", err);
            }

            if let Err(error) = tx.send(BalancerRequest::DecrementServerConnections(address)) {
                log::error!(
                    "failed to send decrement server connections signal: {}",
                    error
                );
            }
        });
    }
}

async fn delegate(
    url: String,
    req: Request<Incoming>,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let parsed_url = url::Url::parse(&url)?;

    let addrs = parsed_url.socket_addrs(|| None)?;
    let stream = TcpStream::connect(addrs[0]).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;

    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            log::error!("Connection failed: {:?}", err);
        }
    });

    let res = sender.send_request(req).await?;

    return Ok(res);
}
