# Deployment & Operations Guide

This guide covers production deployment strategies, networking configuration, container orchestration, environment tuning, and multi-server mesh topologies for **axAudio**.

---

## 1. Environment Variable Reference

Configure axAudio using the following environment variables:

| Variable | Default | Purpose |
| :--- | :---: | :--- |
| `PORT` | `2333` | TCP port for the Axum REST API and WebSocket telemetry. |
| `TOKIO_WORKER_THREADS` | `2` | Number of worker threads for the asynchronous Tokio runtime. Keep at `2` for 1–2 vCPU servers. |
| `AUDIO_BITRATE` | `128000` | Target Opus audio bitrate in bits per second (e.g., `128000` for 128 kbps, `96000` for 96 kbps, `384000` for 384 kbps studio). |
| `RUST_LOG` | `axAudio=info,tower_http=info` | Tracing log filter levels (`trace`, `debug`, `info`, `warn`, `error`). |
| `BGUTIL_POT_URL` | `http://bgutil-pot:4416` | URL of the `bgutil-ytdlp-pot-provider` sidecar for YouTube Proof of Origin tokens. |
| `YTDLP_COOKIES_DIR` | `/app/cookies` | Directory path containing Netscape `.txt` cookie files for automatic round-robin rotation. |
| `YTDLP_COOKIES_POOL` | *(unset)* | Optional comma-separated list of cookie file paths. |
| `YTDLP_COOKIES_PATH` | `/app/cookies.txt` | Fallback single cookie file path. |
| `YTDLP_PLAYLIST_MAX` | `25` | Maximum number of items indexed per playlist batch (`--playlist-end`). |

---

## 2. Docker Compose Deployment (Recommended)

The standard production setup runs `axAudio` alongside the `bgutil-pot` token sidecar.

### `docker-compose.yml`

```yaml
services:
  axaudio:
    image: ghcr.io/shnimlz/axaudio:latest
    container_name: axaudio
    restart: unless-stopped
    # Use host networking for zero-overhead UDP voice packet routing:
    network_mode: "host"
    environment:
      - PORT=2333
      - RUST_LOG=axAudio=info,tower_http=info
      - TOKIO_WORKER_THREADS=2
      - BGUTIL_POT_URL=http://127.0.0.1:4416
      - YTDLP_COOKIES_DIR=/app/cookies
    volumes:
      - ./cookies:/app/cookies
    depends_on:
      - bgutil-pot
    deploy:
      resources:
        limits:
          cpus: '1.50'
          memory: 128M
    healthcheck:
      test: ["CMD", "curl", "-f", "http://127.0.0.1:2333/health"]
      interval: 15s
      timeout: 3s
      retries: 3
      start_period: 5s

  bgutil-pot:
    image: brainicism/bgutil-ytdlp-pot-provider:latest
    container_name: bgutil-pot
    restart: unless-stopped
    network_mode: "host"
    environment:
      - PORT=4416
```

### Cookie Setup
1. Create a `cookies/` directory next to `docker-compose.yml`.
2. Place one or more Netscape cookie files exported from desktop browsers (`account1.txt`, `account2.txt`).
3. Ensure file permissions allow writes so tokens can be refreshed automatically:
   ```bash
   chmod -R a+rwX ./cookies
   ```

Launch the stack:
```bash
docker compose up -d
```

---

## 3. Bare Metal Installation

If running outside Docker on Linux (Debian / Ubuntu / Alpine):

### System Prerequisites

```bash
# Debian / Ubuntu
sudo apt-get update
sudo apt-get install -y cmake pkg-config libopus-dev python3 python3-pip curl unzip

# Install Deno (required by yt-dlp for YouTube EJS signature deobfuscation)
curl -fsSL https://deno.land/install.sh | sh
sudo cp ~/.deno/bin/deno /usr/local/bin/

# Install yt-dlp & PoToken provider
sudo pip3 install --break-system-packages -U yt-dlp bgutil-ytdlp-pot-provider
```

### Compilation

```bash
git clone https://github.com/Shnimlz/axAudio.git
cd axAudio

# Compile optimized release binary with Link-Time Optimization (LTO)
cargo build --release

# The compiled ~8.4 MB binary will be located at:
# ./target/release/axAudio
```

### Systemd Service Unit (`/etc/systemd/system/axaudio.service`)

```ini
[Unit]
Description=axAudio Discord Voice Engine
After=network.target

[Service]
Type=simple
User=axaudio
WorkingDirectory=/opt/axaudio
ExecStart=/opt/axaudio/axAudio
Restart=always
RestartSec=3
LimitNOFILE=65535
Environment=PORT=2333
Environment=TOKIO_WORKER_THREADS=2
Environment=RUST_LOG=axAudio=info,tower_http=info
Environment=YTDLP_COOKIES_DIR=/opt/axaudio/cookies

[Install]
WantedBy=multi-user.target
```

---

## 4. Networking & Voice Transport

Discord voice relies heavily on outbound **UDP** traffic. A common failure in containerized voice engines is dropped audio caused by Docker bridge Network Address Translation (NAT) table saturation.

### Why `network_mode: "host"` is Recommended
1. **Zero NAT Overhead**: Discord voice streams transmit 50 UDP packets every second per active voice stream. In bridge networks, conntrack tables must translate each packet header.
2. **Direct Ephemeral Ports**: Songbird negotiates dynamic high UDP ports with Discord's voice servers. Host networking avoids having to pre-map thousands of UDP port ranges.

If bridge networking must be used, ensure Docker's ephemeral UDP port range is open in host firewall rules (`iptables` / `ufw`).

---

## 5. Multi-Server Mesh & Homelab Topologies

axAudio can be deployed on a dedicated homelab or secondary computing node while your primary bot runs on a cloud VPS.

```
┌─────────────────────────────────┐                 ┌─────────────────────────────────┐
│     Cloud VPS (amayo-vps)       │                 │   Homelab Node (vps-local)      │
│                                 │                 │                                 │
│  ┌───────────────────────────┐  │  Tailscale Mesh │  ┌───────────────────────────┐  │
│  │   Discord Bot Engine      │  │ ──────────────► │  │   axAudio Native Engine   │  │
│  │   (TypeScript / Node.js)  │  │   100.x.x.x     │  │   (Rust Core on Port 2333)│  │
│  └───────────────────────────┘  │                 │  └─────────────┬─────────────┘  │
└─────────────────────────────────┘                 └────────────────┼────────────────┘
                                                                     │ (50 Hz UDP RTP)
                                                                     ▼
                                                         [Discord Voice Gateway]
```

### Private Mesh Peering with Tailscale
By placing `axAudio` and your bot inside a private **Tailscale** network:
- The bot communicates over secure WireGuard mesh tunnels (`http://100.x.x.x:2333`).
- Audio traffic exits directly from the host machine hosting `axAudio` to Discord's media endpoints.
- No public ingress ports or port-forwarding rules need to be opened on the homelab router.

---

## 6. Sizing & Resource Sizing

| Workload | Recommended CPU | Recommended RAM |
| :--- | :---: | :---: |
| **Idle Node** | 0.01 vCPU | **~8–15 MB** |
| **1 Active Voice Guild** | ~1–2% of 1 vCPU | **~18 MB** |
| **10 Active Voice Guilds** | ~10–15% of 1 vCPU | **~35 MB** |
| **50 Active Voice Guilds** | ~0.5 vCPU | **~110 MB** |

Unlike Java alternatives that mandate at least 1 GB of RAM just to boot, axAudio operates with peak stability inside a constrained **128 MB Docker memory limit**.
