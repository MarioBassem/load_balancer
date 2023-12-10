use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use serde_json;
use std::{future::Future, pin::Pin};
use tokio::net::{TcpListener, TcpStream};

// use crossbeam_channel::{Receiver, Sender};
use async_channel::{Receiver, Sender};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Incoming},
    service::Service,
    Request, Response,
};

use crate::balancer::{BalancerError, BalancerRequest, BalancerResponse};
use crate::server::Server;

pub(crate) async fn balancer_api_listener(
    listener: TcpListener,
    tx: Sender<BalancerRequest>,
    rx: Receiver<BalancerResponse>,
) {
    loop {
        let tx = tx.clone();
        let rx = rx.clone();
        let (stream, _) = match listener.accept().await {
            Ok(stream) => stream,
            Err(error) => {
                log::error!("failed to accept connection on unix socket: {}", error);
                continue;
            }
        };

        let io = TokioIo::new(stream);
        let service = service_fn(move |req| process_request(req, tx.clone(), rx.clone()));
        if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
            log::error!("Failed to serve connection: {:?}", err);
        }
    }
}

async fn process_request(
    req: Request<Incoming>,
    tx: Sender<BalancerRequest>,
    rx: Receiver<BalancerResponse>,
) -> Result<Response<String>, Box<dyn std::error::Error + Send + Sync>> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::POST, "/add") => {
            let body = req.collect().await?.to_bytes();
            log::debug!("adding server: {:?}", body);
            let server: Server =
                serde_json::from_slice(&body.iter().cloned().collect::<Vec<u8>>())?;

            tx.send(BalancerRequest::AddServer(server)).await?;
        }
        (&hyper::Method::POST, "/delete") => {
            let body = req.collect().await?.to_bytes();
            let url = String::from_utf8(body.iter().cloned().collect::<Vec<u8>>())?;

            tx.send(BalancerRequest::DeleteServer(url)).await?;
        }
        (&hyper::Method::POST, "/update") => {
            let body = req.collect().await?.to_bytes();
            let server: Server =
                serde_json::from_slice(&body.iter().cloned().collect::<Vec<u8>>())?;

            tx.send(BalancerRequest::UpdateServer(server)).await?;
        }
        (_, _) => {
            return Ok(Response::builder()
                .status(hyper::StatusCode::BAD_REQUEST)
                .body("bad request".to_string())
                .unwrap())
        }
    };

    let resp = match rx.recv().await {
        Ok(BalancerResponse::Ok) => Response::builder()
            .status(hyper::StatusCode::OK)
            .body("".to_string())
            .unwrap(),
        Ok(BalancerResponse::Error(code, message)) => {
            Response::builder().status(code).body(message).unwrap()
        }
        Err(error) => Response::builder()
            .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
            .body(error.to_string())
            .unwrap(),
    };

    return Ok(resp);
}
