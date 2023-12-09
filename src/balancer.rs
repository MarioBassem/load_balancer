use crate::server::Server;
use crate::{api_service, balancer_service};
use crossbeam_channel::{bounded, select, Receiver, Sender};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, BodyStream};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::StatusCode;
use hyper::{
    body::{Body, Incoming},
    Request, Response,
};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use url::ParseError;

pub struct Balancer {
    servers: HashMap<String, Server>,
    port: u16,
    api_port: u16,
}

#[derive(Debug)]
pub enum BalancerError {
    IO(std::io::Error),
    MyError(String),
    ConfigError(serde_yaml::Error),
    ParseError(ParseError),
}

#[derive(Debug)]
pub enum BalancerRequest {
    DecrementServerConnections(String),
    AddServer(Server),
    DeleteServer(String),
    UpdateServer(Server),
}

#[derive(Debug)]
pub enum BalancerResponse {
    Ok,
    Error(StatusCode, String), // status code, error message
}

pub fn new(path: Option<String>, port: u16, api_port: u16) -> Result<Balancer, BalancerError> {
    if let Some(p) = path {
        let map = Balancer::read_config_file(p)?;
        return Ok(Balancer {
            servers: map,
            port,
            api_port,
        });
    }

    return Ok(Balancer {
        servers: HashMap::new(),
        port,
        api_port,
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

    pub async fn listen(&mut self) -> Result<(), BalancerError> {
        // run http server
        // balancer listens for incoming requests
        // balancer decides which server to reroute request to
        // balancer updates chosen server status
        let addr: SocketAddr = ([127, 0, 0, 1], self.port).into();

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| BalancerError::IO(e))?;
        log::info!("Listening on http://{}", addr);

        let api_addr: SocketAddr = ([127, 0, 0, 1], self.api_port).into();
        let api_listener = TcpListener::bind(api_addr)
            .await
            .map_err(|e| BalancerError::IO(e))?;
        log::info!("API Listening on http://{}", api_addr);

        let (request_tx, request_rx) = bounded(0);
        let (balancer_tx, worker_rx) = bounded(0);
        let (api_tx, api_rx) = bounded(0);

        tokio::spawn(api_service::balancer_api_listener(
            api_listener,
            request_tx.clone(),
            api_rx.clone(),
        ));
        tokio::spawn(balancer_service::balancer_listener(
            listener,
            request_tx.clone(),
            worker_rx.clone(),
        ));

        loop {
            log::debug!("balancer waiting for requests...");
            let next = match self.choose_next_server() {
                Ok(url) => url,
                Err(error) => {
                    // should never happen
                    log::error!("failed to get next server address: {}", error);
                    "".to_string()
                }
            };

            select! {
                recv(request_rx) -> receive_result =>{
                    let req = match receive_result{
                        Ok(req) => req,
                        Err(error) => {
                            log::error!("failed to receive balancer request: {}", error);
                            continue;
                        }
                    };

                    log::debug!("got request: {:?}", req);
                    let res = match req {
                        BalancerRequest::DecrementServerConnections(url) => {
                            self.decrement_server_connections(url)
                        },
                        BalancerRequest::AddServer(server) => {
                            self.add_server(server)
                        },
                        BalancerRequest::DeleteServer(url) =>{
                            self.delete_server(url)
                        },
                        BalancerRequest::UpdateServer(server) =>{
                            self.modify_server(server)
                        },
                    };

                    if let Err(error) = res{
                        log::error!("failed to perfrom request: {}", error)
                    }
                },
                send(balancer_tx, next.clone()) -> send_result => {
                    if let Err(error) = send_result{
                        log::error!("failed to send next server url signal: {}", error);
                    }

                    if let Err(error) = self.increment_server_connections(next){
                        log::error!("failed to increment server connections: {}", error);
                    }
                },
            };
        }
    }

    fn add_server(&mut self, server: Server) -> Result<(), BalancerError> {
        if self.servers.contains_key(&server.url) {
            return Err(BalancerError::MyError(format!(
                "a server with url {} already exists",
                server.url
            )));
        }

        self.servers.insert(server.url.clone(), server);

        Ok(())
    }

    fn delete_server(&mut self, url: String) -> Result<(), BalancerError> {
        // prevent deleting all servers
        if !self.servers.contains_key(&url) {
            return Err(BalancerError::MyError(format!(
                "server with url {} not found",
                url
            )));
        }

        if self.servers.len() == 1 {
            return Err(BalancerError::MyError(
                "balancer cannot delete the only active server".to_string(),
            ));
        }

        self.servers.remove_entry(&url);

        Ok(())
    }

    fn modify_server(&mut self, server: Server) -> Result<(), BalancerError> {
        // prevent disabling all servers
        todo!()
    }

    fn choose_next_server(&self) -> Result<String, BalancerError> {
        let least_connections_server = self.servers.values().min_by(|x, y| x.cmp(y));

        let server = match least_connections_server {
            Some(server) => server,
            None => {
                return Err(BalancerError::MyError(format!(
                    "balancer has 0 working servers"
                )))
            }
        };

        return Ok(server.url.clone());
    }

    fn increment_server_connections(&mut self, url: String) -> Result<(), BalancerError> {
        match self.servers.get_mut(&url) {
            Some(server) => server.connections += 1,
            None => {
                return Err(BalancerError::MyError(format!(
                    "server with url {} not found",
                    url
                )))
            }
        }

        Ok(())
    }

    fn decrement_server_connections(&mut self, url: String) -> Result<(), BalancerError> {
        let server = self
            .servers
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

#[cfg(test)]
mod test {
    use std::{collections::HashMap, os::unix::net::SocketAddr};

    use crate::server::Server;

    use super::Balancer;

    #[test]
    fn deserialize_servers() {
        let text = "
- name: server1
  url: http://127.0.0.1:3001
  disabled: false
  weight: 1
  health_check_period: 5

- name: server2
  url: http://127.0.0.1:3002
  disabled: TRUE
  weight: 2
  health_check_period: 7
        ";
        let c = std::io::Cursor::new(text);
        let got = Balancer::get_servers(c).unwrap();
        let want = HashMap::from([
            (
                "http://127.0.0.1:3001".to_string(),
                Server {
                    url: "http://127.0.0.1:3001".to_string(),
                    connections: 0,
                    disabled: false,
                    name: "server1".to_string(),
                    weight: 1,
                    health_check_period: 5,
                    healthy: false,
                },
            ),
            (
                "http://127.0.0.1:3002".to_string(),
                Server {
                    url: "http://127.0.0.1:3002".to_string(),
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
