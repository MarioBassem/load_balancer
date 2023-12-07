# Load Balancer

This is a load balancer. I build it for learning purposes, part of the coding challenges from <https://codingchallenges.fyi/>

## How it works

- it balances load between configured servers using `least connections` method
- it performs a periodic health check on configured servers, unhealthy servers are removed from available servers
- it allows users to provide the health check period
- health checks are performed by performing an http GET request for `/` to each server, if a server responds with a status code outside the range from `200` to `399`, the health check fails.
- servers are defined through JSON configuration files. each file specifies the following:
  - server endpoint (hostname:port) (required)
  - health check path
  - health check period
  - server weight
- it allows to register new servers.

### Components

- Balancer
  - contains server configurations
  - mainpulates configurations
  - exposes an api to manipulate server configs
  - performs health checks for each server periodically
  - balances between incoming requests using `least connections` method and server weights
