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
    servers: Mutex<HashMap<String, Server>>,
    port: u16,
}

#[derive(Debug)]
pub enum BalancerError {
    IO(std::io::Error),
    Lock(String),
    MyError(String),
    ConfigError(serde_yaml::Error),
    ParseError(url::ParseError),
}

pub fn new(path: Option<String>, port: u16) -> Result<Balancer, BalancerError> {
    if let Some(p) = path {
        let map = Balancer::read_config_file(p)?;
        return Ok(Balancer {
            servers: Mutex::new(map),
            port: port,
        });
    }

    return Ok(Balancer {
        servers: Mutex::new(HashMap::new()),
        port: port,
    });
}

impl Balancer {
    fn read_config_file(path: String) -> Result<HashMap<String, Server>, BalancerError> {
        let f = std::fs::File::open(path).map_err(|error| BalancerError::IO(error))?;
        let map = Balancer::get_servers(f)?;

        return Ok(map);
    }

    fn get_servers<R: std::io::Read>(r: R) -> Result<HashMap<String, Server>, BalancerError> {
        let d: Vec<Server> =
            serde_yaml::from_reader(r).map_err(|error| BalancerError::ConfigError(error))?;
        let map: Result<HashMap<String, Server>, BalancerError> = d
            .into_iter()
            .map(|v| {
                if let Err(error) = url::Url::parse(&v.url) {
                    return Err(BalancerError::ParseError(error));
                }
                Ok((v.url.clone(), v))
            })
            .collect();

        return Ok(map?);
    }

    pub async fn listen(self) -> Result<(), BalancerError> {
        // run http server
        // balancer listens for incoming requests
        // balancer decides which server to reroute request to
        // balancer updates chosen server status
        let addr: SocketAddr = ([127, 0, 0, 1], self.port).into();

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

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use crate::server::Server;

    use super::Balancer;

    #[test]
    fn deserialize_servers() {
        let text = "
- name: server1
  url: 127.0.0.1:3001
  disabled: false
  weight: 1
  health_check_period: 5

- name: server2
  url: 127.0.0.1:3002
  disabled: TRUE
  weight: 2
  health_check_period: 7
        ";
        let c = std::io::Cursor::new(text);

        let got = Balancer::get_servers(c).unwrap();
        let want = HashMap::from([
            (
                "127.0.0.1:3001".to_string(),
                Server {
                    url: "127.0.0.1:3001".to_string(),
                    connections: 0,
                    disabled: false,
                    name: "server1".to_string(),
                    weight: 1,
                    health_check_period: 5,
                    healthy: false,
                },
            ),
            (
                "127.0.0.1:3002".to_string(),
                Server {
                    url: "127.0.0.1:3002".to_string(),
                    connections: 0,
                    disabled: true,
                    name: "server2".to_string(),
                    weight: 2,
                    health_check_period: 7,
                    healthy: false,
                },
            ),
        ]);

        assert_eq!(got, want)
    }
}
