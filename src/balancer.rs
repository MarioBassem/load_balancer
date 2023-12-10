use crate::health_check_service::{health_check_service, HealthCheckRequest, HealthReport};
use crate::server::Server;
use crate::{api_service, balancer_service};
use async_channel::{bounded, Sender};
use hyper::StatusCode;
use std::collections::HashMap;
use std::fmt::Display;
use std::net::SocketAddr;
use tokio::net::TcpListener;
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

impl Display for BalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BalancerError::ConfigError(s) => {
                write!(f, "Config Error: {}", s)?;
            }
            BalancerError::IO(s) => {
                write!(f, "IO Error: {}", s)?;
            }
            BalancerError::MyError(s) => {
                write!(f, "Balancer Error: {}", s)?;
            }
            BalancerError::ParseError(s) => {
                write!(f, "Parsing Error: {}", s)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum BalancerRequest {
    AddServer(Server),
    DeleteServer(String),
    UpdateServer(Server),
}

#[derive(Debug)]
pub enum BalancerResponse {
    Ok,
    Error(StatusCode, String), // status code, error message
}

impl Balancer {
    pub fn new(path: Option<String>, port: u16, api_port: u16) -> Result<Balancer, BalancerError> {
        if let Some(p) = path {
            let map = Balancer::read_config_file(p)?;
            return Ok(Balancer {
                servers: map,
                port,
                api_port,
            });
        }

        Ok(Balancer {
            servers: HashMap::new(),
            port,
            api_port,
        })
    }

    fn read_config_file(path: String) -> Result<HashMap<String, Server>, BalancerError> {
        let f = std::fs::File::open(path).map_err(BalancerError::IO)?;
        let map = Balancer::get_servers(f)?;

        Ok(map)
    }

    fn get_servers<R: std::io::Read>(r: R) -> Result<HashMap<String, Server>, BalancerError> {
        let d: Vec<Server> = serde_yaml::from_reader(r).map_err(BalancerError::ConfigError)?;
        let map: Result<HashMap<String, Server>, BalancerError> = d
            .into_iter()
            .map(|v| {
                if let Err(error) = url::Url::parse(&v.url) {
                    return Err(BalancerError::ParseError(error));
                }
                Ok((v.url.clone(), v))
            })
            .collect();

        map
    }

    pub async fn listen(&mut self) -> Result<(), BalancerError> {
        // run http server
        // balancer listens for incoming requests
        // balancer decides which server to reroute request to
        // balancer updates chosen server status
        let addr: SocketAddr = ([127, 0, 0, 1], self.port).into();

        let listener = TcpListener::bind(addr).await.map_err(BalancerError::IO)?;
        log::info!("Listening on http://{}", addr);

        let api_addr: SocketAddr = ([127, 0, 0, 1], self.api_port).into();
        let api_listener = TcpListener::bind(api_addr)
            .await
            .map_err(BalancerError::IO)?;
        log::info!("API Listening on http://{}", api_addr);

        let (balancer_request_tx, balancer_request_rx) = bounded(1);
        let (balancer_response_tx, balancer_response_rx) = bounded(1);
        tokio::spawn(api_service::balancer_api_listener(
            api_listener,
            balancer_request_tx,
            balancer_response_rx,
        ));

        let (decrement_sig_request_tx, decrement_sig_request_rx) = bounded(1);
        let (next_server_tx, next_server_rx) = bounded(1);
        tokio::spawn(balancer_service::balancer_listener(
            listener,
            decrement_sig_request_tx,
            next_server_rx,
        ));

        let (health_check_request_tx, health_check_request_rx) = bounded(1);
        let (health_report_tx, health_report_rx) = bounded(1);
        tokio::spawn(health_check_service(
            self.servers
                .values()
                .map(|v| (v.url.clone(), v.health_check_period))
                .collect(),
            health_report_tx,
            health_check_request_rx,
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

            tokio::select! {
                  receive_result = decrement_sig_request_rx.recv() =>{
                    let req = match receive_result{
                        Ok(req) => req,
                        Err(error) =>{
                            log::error!("failed to receive decrement connections signal: {}", error);
                            continue;
                        }
                    };

                    log::debug!("received decrement request: {:?}", req);
                    if let Err(error) = self.decrement_server_connections(req.0){
                        log::error!("failed to process decrement signal: {}", error);
                    }
                }
                 receive_result = balancer_request_rx.recv() =>{
                    let req = match receive_result{
                        Ok(req) => req,
                        Err(error) => {
                            log::error!("failed to receive balancer request: {}", error);
                            continue;
                        }
                    };

                    log::debug!("received balancer request: {:?}", req);
                    let response =  match self.process_balancer_request(req, health_check_request_tx.clone()).await{
                        Ok(()) => BalancerResponse::Ok,
                        Err(error) => error,
                    };
                    log::debug!("sending balancer response: {:?}", response);

                    if let Err(error) = balancer_response_tx.send(response).await{
                        log::error!("failed to send balancer responser: {}", error);
                    }
                },
                 receive_result = health_report_rx.recv() =>{
                    let report = match receive_result{
                        Ok(report) => report,
                        Err(error) =>{
                            log::error!("failed to receive unhealthy server signal: {}", error);
                            continue;
                        }
                    };

                    log::debug!("received health report: {:?}", report);
                    if let Err(error) = self.process_health_report(report){
                        log::error!("failed to process health report: {}",  error);
                    };
                },
                send_result = next_server_tx.send(next.clone())=> {
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

    async fn process_balancer_request(
        &mut self,
        request: BalancerRequest,
        health_check_request_tx: Sender<HealthCheckRequest>,
    ) -> Result<(), BalancerResponse> {
        match request {
            BalancerRequest::AddServer(server) => {
                log::debug!("balancer trying to add server: {:?}", server);
                let (url, period) = (server.url.clone(), server.health_check_period);
                self.add_server(server)
                    .map_err(|e| BalancerResponse::Error(StatusCode::BAD_REQUEST, e.to_string()))?;

                log::debug!("sending add server to worker");
                health_check_request_tx
                    .send(HealthCheckRequest::Start(url, period))
                    .await
                    .map_err(|e| {
                        BalancerResponse::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                    })?;
            }
            BalancerRequest::DeleteServer(url) => {
                self.delete_server(url.clone())
                    .map_err(|e| BalancerResponse::Error(StatusCode::BAD_REQUEST, e.to_string()))?;

                health_check_request_tx
                    .send(HealthCheckRequest::Stop(url))
                    .await
                    .map_err(|e| {
                        BalancerResponse::Error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                    })?;
            }
            BalancerRequest::UpdateServer(server) => {
                self.modify_server(server)
                    .map_err(|e| BalancerResponse::Error(StatusCode::BAD_REQUEST, e.to_string()))?;
            }
        }

        Ok(())
    }

    fn process_health_report(&mut self, report: HealthReport) -> Result<(), BalancerError> {
        match report {
            HealthReport::Unhealhty(url) => self.update_server_health(&url, false),
            HealthReport::Healthy(url) => self.update_server_health(&url, true),
        }
    }

    fn add_server(&mut self, server: Server) -> Result<(), BalancerError> {
        log::debug!("add_server");
        if self.servers.contains_key(&server.url) {
            log::debug!("contains key");
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

    fn modify_server(&mut self, new_server: Server) -> Result<(), BalancerError> {
        // prevent disabling all servers
        let cur_server = match self.servers.get_mut(&new_server.url) {
            None => {
                return Err(BalancerError::MyError(format!(
                    "failed to find server with url {}",
                    new_server.url
                )))
            }
            Some(s) => s,
        };

        cur_server.name = new_server.name;
        cur_server.health_check_period = new_server.health_check_period;
        cur_server.weight = new_server.weight;

        Ok(())
    }

    fn choose_next_server(&self) -> Result<String, BalancerError> {
        let least_connections_server = self.servers.values().min_by(|x, y| x.cmp(y));

        let server = match least_connections_server {
            Some(server) => server,
            None => {
                return Err(BalancerError::MyError(
                    "balancer has 0 working servers".to_string(),
                ))
            }
        };

        Ok(server.url.clone())
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

    fn update_server_health(&mut self, url: &str, healthy: bool) -> Result<(), BalancerError> {
        let server = self
            .servers
            .get_mut(url)
            .ok_or(BalancerError::MyError(format!("server {} not found", url)))?;

        server.healthy = healthy;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use async_channel::bounded;

    use crate::{health_check_service::HealthCheckRequest, server::Server};

    use super::Balancer;

    #[test]
    fn deserialize_servers() {
        let text = "
- name: server1
  url: http://127.0.0.1:3001
  weight: 1
  health_check_period: 5

- name: server2
  url: http://127.0.0.1:3002
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
                    name: "server2".to_string(),
                    weight: 2,
                    health_check_period: 7,
                    healthy: false,
                },
            ),
        ]);

        assert_eq!(got, want)
    }

    #[test]
    fn new_empty_balancer() {
        let balancer = Balancer::new(None, 3000, 3001).unwrap();
        assert_eq!(balancer.port, 3000);
        assert_eq!(balancer.api_port, 3001);
        assert_eq!(balancer.servers, HashMap::new());
    }

    #[test]
    fn new_balancer() {
        let balancer = Balancer::new(Some("config.yaml".to_string()), 3000, 5000).unwrap();
        assert_eq!(balancer.port, 3000);
        assert_eq!(balancer.api_port, 5000);
        assert_eq!(
            balancer.servers,
            HashMap::from([
                (
                    "http://127.0.0.1:3001".to_string(),
                    Server {
                        url: "http://127.0.0.1:3001".to_string(),
                        connections: 0,
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
                        name: "server2".to_string(),
                        weight: 2,
                        health_check_period: 7,
                        healthy: false,
                    },
                ),
            ])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_balancer_request_add() {
        let mut balancer = Balancer::new(Some("config.yaml".to_string()), 3000, 3001).unwrap();
        let (tx, rx) = bounded(1);
        let url = "http://128.1.1.3:4000".to_string();
        let url_clone = url.clone();
        let request = crate::balancer::BalancerRequest::AddServer(Server {
            url: url.clone(),
            name: "server3".to_string(),
            weight: 5,
            health_check_period: 9,
            connections: 0,
            healthy: false,
        });

        let rx_clone = rx.clone();
        tokio::spawn(async move {
            let sig = rx_clone.recv().await.unwrap();
            match sig {
                HealthCheckRequest::Start(u, period) => {
                    assert_eq!(u, url.clone());
                    assert_eq!(period, 9);
                }
                _ => {
                    panic!(
                        "balancer did not send a health check start request, but sent {:?}",
                        sig
                    );
                }
            }
        });

        balancer
            .process_balancer_request(request, tx.clone())
            .await
            .unwrap();

        assert!(
            balancer.servers.get(&url_clone).unwrap()
                == &Server {
                    url: url_clone,
                    name: "server3".to_string(),
                    weight: 5,
                    health_check_period: 9,
                    connections: 0,
                    healthy: false,
                }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_balancer_request_delete() {
        let mut balancer = Balancer::new(Some("config.yaml".to_string()), 3000, 3001).unwrap();
        let (tx, rx) = bounded(1);
        let url = "http://127.0.0.1:3001".to_string();
        let url_clone = url.clone();
        let request = crate::balancer::BalancerRequest::DeleteServer(url.clone());

        let rx_clone = rx.clone();
        tokio::spawn(async move {
            let sig = rx_clone.recv().await.unwrap();
            match sig {
                HealthCheckRequest::Stop(u) => {
                    assert_eq!(u, url.clone());
                }
                _ => {
                    panic!(
                        "balancer did not send a health check start request, but sent {:?}",
                        sig
                    );
                }
            }
        });

        balancer
            .process_balancer_request(request, tx.clone())
            .await
            .unwrap();

        assert!(!balancer.servers.contains_key(&url_clone));
        assert_eq!(balancer.servers.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_balancer_request_update() {
        let mut balancer = Balancer::new(Some("config.yaml".to_string()), 3000, 3001).unwrap();
        let (tx, _) = bounded(1);
        let url = "http://127.0.0.1:3001".to_string();
        let request = crate::balancer::BalancerRequest::UpdateServer(Server {
            url: url.clone(),
            name: "server4".to_string(),
            weight: 10,
            health_check_period: 20,
            connections: 10,
            healthy: true,
        });

        balancer
            .process_balancer_request(request, tx.clone())
            .await
            .unwrap();

        assert_eq!(balancer.servers.len(), 2);
        assert_eq!(
            balancer.servers.get(&url).unwrap(),
            &Server {
                url: url.clone(),
                name: "server4".to_string(),
                weight: 10,
                health_check_period: 20,
                connections: 0,
                healthy: false,
            }
        );
    }
}
