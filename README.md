# Load Balancer

This is a load balancer. I build it for learning purposes, part of the coding challenges from <https://codingchallenges.fyi/>

## Description

The load balancer balances load between configured servers using `least connections` method. It performs a periodic health check on configured servers, healthy servers are prefered to unhealthy servers when delegating requests. Health checks are performed by firing an http GET request for `/` to each server, if a server responds with a status code outside the range from `200` to `399`, the health check fails. It also allows to register, delete, and modify servers through an http api, and a simple command line interface.

### Components

- Balancer
  - contains server configurations
  - exposes an api to manipulate server configs
  - fires a health checks service to monitor and report servers' health
  - balances between incoming requests using `least connections` method and server weights

- HTTP API
  - used to manipulated server configs
  - it uses an http endpoint to accept requests
  - available endoints are:
    - `POST` 127.0.0.1:`API_PORT`/add
      - request body:
  
      ```json
        {
          "url": "string",
          "name": "string",
          "weight": "uint32",
          "health_check_period": "uint32"
        }
      ```

    - `POST` 127.0.0.1:`API_PORT`/delete
      - request body:
        - `server_url`
    - `POST` 127.0.0.1:`API_PORT`/update
      - request body:

        ```json
        {
          "url": "string",
          "name": "string",
          "weight": "uint32",
          "health_check_period": "uint32"
        }
        ```

- Health Check Service
  - spawns a separate health check thread for each server.
  - listens and reacts to modifications in monitored servers, so a new health check thread will be spawned for newly added servers, deleted servers will have their threads aborted, and health check periods for current servers should be reflected if modified.

- CLI
  - the app exposes a cli to interact with the balancer
  - available commands:
    - listen:
      - starts the load balancer with the specified configurations
      - command arguments:
        - `--config`: specified the path of a yaml file which contains servers configurations
        - `--port`: this is the load balancer's port. incoming requests on this port will be delegated to the suitable server. defaults to `80`
        - `--api-port`: this is the api port for the load balancer: requests to modify server configs will be accepted on this port. defaults to `8000`
        - `--debug`: enable debug logging
    - add-server:
      - adds a new server to the load balancer
      - command arguments:
        - `--url`: server url, has to be unique.
        - `--name`: server name
        - `--weight`: server weight, higher values result in more requests delegated to this server. defaults to 1
        - `--health-check-period`: balancer will perform a health check on this server each specified period. defaults to 10
        - `--api-port`: balancer's api port. defaults to `8000`
    - delete-server:
      - deletes a monitored server
      - command arguments:
        - `--url`: server url
        - `--api-port`: balancer's api port: defaults to `8000`
    - update-server:
      - updates a currently monitored server
      - command arguments:
        - `--url`: server url, has to be already added.
        - `--name`: server name
        - `--weight`: server weight, higher values result in more requests delegated to this server. defaults to 1
        - `--health-check-period`: balancer will perform a health check on this server each specified period. defaults to 10
        - `--api-port`: balancer's api port. defaults to `8000`
