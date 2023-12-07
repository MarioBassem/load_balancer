use crate::server::Server;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming as IncomingBody, Request, Response};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

pub struct Balancer {
    // servers: Vec<Server>,
    servers: Mutex<HashMap<String, Server>>,
}

#[derive(Debug)]
pub enum BalancerError {
    IO(std::io::Error),
    Lock(String),
    MyError(String),
}

pub fn new() -> Balancer {
    let mut servers = HashMap::new();
    servers.insert(
        "127.0.0.1:3001".to_string(),
        Server {
            url: "127.0.0.1:3001".to_string(),
            connections: 0,
            name: "server1".to_string(),
        },
    );
    servers.insert(
        "127.0.0.1:3002".to_string(),
        Server {
            url: "127.0.0.1:3002".to_string(),
            connections: 0,
            name: "server2".to_string(),
        },
    );
    return Balancer {
        servers: Mutex::new(servers),
    };
}

impl Balancer {
    pub async fn listen(self) -> Result<(), BalancerError> {
        // run http server
        // balancer listens for incoming requests
        // balancer decides which server to reroute request to
        // balancer updates chosen server status
        let addr: SocketAddr = ([127, 0, 0, 1], 3000).into();

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| BalancerError::IO(e))?;
        log::info!("Listening on http://{}", addr);

        let balancer_ref = Arc::new(self);
        loop {
            let (stream, _) = listener.accept().await.map_err(|e| BalancerError::IO(e))?;
            let io = TokioIo::new(stream);
            let cl = balancer_ref.clone();
            tokio::task::spawn(async move {
                let url = match cl.assign_server() {
                    Ok(url) => url,
                    Err(error) => {
                        log::error!("failed to get server url: {:?}", error);
                        return;
                    }
                };

                let url_clone = url.clone();
                let service = service_fn(move |req| delegate(url.clone(), req));

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    log::error!("Failed to serve connection: {:?}", err);
                }

                if let Err(error) = cl.decrement_server_connections(url_clone) {
                    log::error!("failed to decrement server connections: {:?}", error);
                    return;
                }
            });
        }
    }

    fn assign_server(&self) -> Result<String, BalancerError> {
        let mut servers_guard = self
            .servers
            .lock()
            .map_err(|error| BalancerError::Lock(error.to_string()))?;

        let least_connections_server =
            servers_guard
                .values_mut()
                .min_by(|x, y| x.cmp(y))
                .ok_or(BalancerError::MyError(
                    "failed to find min connections server: no servers found".to_string(),
                ))?;

        least_connections_server.connections += 1;

        log::debug!(
            "server {} has {} open connections",
            least_connections_server.name,
            least_connections_server.connections
        );

        return Ok(least_connections_server.url.clone());
    }

    fn decrement_server_connections(&self, url: String) -> Result<(), BalancerError> {
        let mut servers_guard = self
            .servers
            .lock()
            .map_err(|error| BalancerError::Lock(error.to_string()))?;

        let server = servers_guard
            .get_mut(&url)
            .ok_or(BalancerError::MyError(format!(
                "failed to find server with url {}",
                url
            )))?;

        server.connections -= 1;

        log::debug!(
            "server {} has {} open connections",
            server.name,
            server.connections
        );

        Ok(())
    }
}

async fn delegate(
    url: String,
    req: Request<IncomingBody>,
) -> Result<Response<IncomingBody>, Box<dyn std::error::Error + Send + Sync>> {
    let stream = TcpStream::connect(url).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;

    // shouldn't this be closed??
    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            log::error!("Connection failed: {:?}", err);
        }
    });

    let res = sender.send_request(req).await?;

    return Ok(res);
}
