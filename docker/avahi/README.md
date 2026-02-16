# mDNS Service Advertisement

Avahi sidecar that advertises the Wyoming satellite as `_wyoming._tcp` on your local network, enabling Home Assistant auto-discovery.

## Usage

```bash
docker compose --profile mdns up -d
```

## Configuration

| Environment Variable | Default | Description |
|---|---|---|
| `SATELLITE_NAME` | `Wyoming Satellite` | Service name visible in discovery |
| `SATELLITE_PORT` | `10700` | Port the satellite listens on |

## Requirements

- `network_mode: host` is required — mDNS uses multicast on port 5353, which doesn't traverse Docker bridge networks.
- The satellite itself must be reachable on the advertised port from the host network.

## Verifying

From another machine on the same network:

```bash
avahi-browse -r _wyoming._tcp
```
