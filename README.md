# nostrd

A high-performance [Nostr](https://github.com/nostr-protocol/nostr) relay server
written in Rust.

> [!IMPOTANT]  
> **Beta software.** This relay is under active development. It is stable and
> production-tested, but APIs and config options may change before 1.0.
> Bug reports and pull requests are welcome.

> [!NOTE]  
> **Support the project.** If you find nostrd useful, please consider sending a
> zap or BTC donation:
>
> **Zap:**  
> `noxiousnexus24@walletofsatoshi.com`
>  
> **BTC:**  
> `19AtehSNENNE6jXF9UWvz2hH8GdCp6UEn`

## Features

- **Blazingly fast** — built on LMDB (memory-mapped zero-copy storage) with
  multi-indexed event lookup and real-time broadcast
- **Crash-proof** — catch-unwind guards, panic logging with backtraces, LMDB ACID
  transactions, graceful shutdown on SIGTERM
- **Memory efficient** — `Arc<Event>` shared references eliminate deep clones,
  bounded broadcast ring buffer with backpressure, configurable connection caps
  and idle timeouts
- **Rate-limited** — per-connection event submission limiting prevents flooding
- **Daemonizable** — runs as a background daemon with PID file and log rotation
- **Fully configurable** — TOML configuration with sensible defaults

## Supported NIPs

| NIP | Description |
|-----|-------------|
| [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md) | Basic protocol flow (EVENT, REQ, CLOSE, EOSE, OK, NOTICE, CLOSED) |
| [NIP-09](https://github.com/nostr-protocol/nips/blob/master/09.md) | Event deletion (kind 5) |
| [NIP-11](https://github.com/nostr-protocol/nips/blob/master/11.md) | Relay information document |
| [NIP-12](https://github.com/nostr-protocol/nips/blob/master/12.md) | Generic tag queries |
| [NIP-18](https://github.com/nostr-protocol/nips/blob/master/18.md) | Reposts (kind 6, 16) |
| [NIP-19](https://github.com/nostr-protocol/nips/blob/master/19.md) | bech32-encoded entities (utility, not used by relay protocol) |
| [NIP-23](https://github.com/nostr-protocol/nips/blob/master/23.md) | Long-form content (kind 30023) |
| [NIP-25](https://github.com/nostr-protocol/nips/blob/master/25.md) | Reactions (kind 7, 17) |
| [NIP-28](https://github.com/nostr-protocol/nips/blob/master/28.md) | Public chat / channel events (kinds 40-44) |
| [NIP-40](https://github.com/nostr-protocol/nips/blob/master/40.md) | Expiration timestamp |
| [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md) | AUTH (client authentication) |
| [NIP-45](https://github.com/nostr-protocol/nips/blob/master/45.md) | COUNT (event count queries) |
| [NIP-77](https://github.com/nostr-protocol/nips/blob/master/77.md) | Negentropy sync |

## Quick Start

### Build from source

```bash
cargo build --release
```

The binary is at `target/release/nostrd`.

### Build with Docker

```bash
# amd64 / x86_64
docker build -f Dockerfile.amd64 -t nostrd:latest .

# arm64 / aarch64 (Apple Silicon, AWS Graviton)
docker build -f Dockerfile.aarch64 -t nostrd:latest-arm64 .
```

Extract the binary:

```bash
docker create --name nostrd-tmp nostrd:latest
docker cp nostrd-tmp:/usr/local/bin/nostrd ./nostrd
docker rm nostrd-tmp
```

### Start the relay

```bash
# Copy the sample config and edit to your needs
cp nostrd.toml.sample nostrd.toml

# Daemonized (background), reads nostrd.toml, logs to ./data/nostrd.log
nostrd start

# Foreground (stdout/stderr, Ctrl+C to stop)
nostrd --foreground

# With custom config and data directory
nostrd -c /etc/nostrd/config.toml -d /var/lib/nostrd

# Verbose logging
nostrd -V        # debug
nostrd -VV       # trace
```

### Manage the relay

```bash
nostrd stop      # Graceful shutdown via SIGTERM
nostrd restart   # Stop + start
nostrd stats     # Show event count and running status
```

### Connect from a client

The relay exposes both WebSocket and HTTP on a single endpoint:

```
ws://localhost:80/     # WebSocket (Nostr protocol)
http://localhost:80/   # HTTP GET → NIP-11 relay info (Accept: application/nostr+json)
```

## Configuration

All options are set in `nostrd.toml` (TOML format). Copy `nostrd.toml.sample`
as a starting point. Defaults are shown below.

```toml
# Network
listen_addr = "0.0.0.0:80"

# Relay Identity (NIP-11)
relay_name = "nostrd"
relay_description = "A Nostr relay server"
# relay_pubkey = ""
# relay_contact = ""
# relay_icon = ""

# Event Validation
max_event_age_days = 30
max_event_tags = 2000
max_content_length = 100000

# Subscription Limits
max_subscription_filters = 10
max_subscriptions_per_client = 20

# Authentication (NIP-42)
auth_required = false
nip42_enabled = true

# Performance
lmdb_map_size_gb = 256       # LMDB virtual address space (not physical RAM)
broadcast_channel_size = 4096 # Event broadcast ring buffer
max_connections = 1000        # Max concurrent WebSocket connections
max_query_candidates = 10000  # Max events scanned per query
max_ws_message_size = 524288  # Max WebSocket message size (bytes)

# Resource Protection
connection_timeout_secs = 300 # Idle connection timeout
max_sessions = 100000         # Max in-memory sessions
max_events_per_sec = 100      # Per-connection event rate limit
max_req_result_limit = 5000   # Max results returned per REQ
```

All numeric limits must be greater than zero (enforced at startup).

## Architecture

```
WebSocket Client
     │
     ▼
┌──────────────────────────────────────┐
│  Axum HTTP/WebSocket Server          │
│  (connection limiting, idle timeout)  │
└──────────┬───────────────────────────┘
           │
     ┌─────▼──────┐
     │ NIP Router │  (pre-checks: kind validation, expiration, format)
     └─────┬──────┘
           │
    ┌──────▼───────┐
    │  Event Store │  (LMDB with 5 indexes)
    │  - events    │
    │  - author    │  ──── real-time broadcast ───► subscribers
    │  - kind      │
    │  - tag       │
    │  - time      │
    └──────────────┘
```

### Memory Model

Every event is stored once as `Arc<Event>`. Broadcasting to N subscribers
increments a reference count rather than deep-cloning. The LMDB storage uses
virtual address space only; physical RAM grows with actual stored data.

### Crash Resilience

- **Global panic hook** captures backtraces to stderr and `nostrd.panic.log`
- **catch_unwind** wraps the server runtime — panics log and shut down
  gracefully instead of aborting
- **Per-connection isolation** — axum spawns each connection in its own task;
  a single connection panic cannot crash other connections
- **LMDB ACID** — all writes are transactional; errors roll back automatically
- **Rate limiter** — prevents event flooding from overwhelming memory
- **Connection cap** — `max_sessions` rejects new connections when full

## Testing

```bash
cargo test        # Unit tests + 23 integration tests

cargo clippy      # Lint (zero warnings maintained)
```

Integration tests spawn real relay instances on unique ports with temporary
LMDB stores, covering event submission, queries, broadcast, COUNT, deletion,
AUTH, protected events, replaceable events, deduplication, reactions, reposts,
and generic tag filters.

## Acknowledgments

Built with:

- **[LMDB](https://symas.com/lmdb/)** (via [heed](https://github.com/meilisearch/heed)) — the
  lightning-fast embedded database that makes zero-copy queries possible
- **[Tokio](https://tokio.rs/)** — async runtime powering thousands of concurrent connections
- **[Axum](https://github.com/tokio-rs/axum)** — ergonomic WebSocket and HTTP handling
- **[secp256k1](https://github.com/rust-bitcoin/rust-secp256k1)** — Schnorr signature verification
- **[Nostr](https://github.com/nostr-protocol/nostr)** — the decentralized protocol that makes it
  all worthwhile

Thanks to all Nostr NIP authors and the relay operator community for feedback and testing.

## License

MIT
