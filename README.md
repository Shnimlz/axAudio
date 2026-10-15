# ⚡ axAudio

**Ultra-lightweight, Bare-Metal Discord Native Audio Engine in Rust.**  
Designed as a zero-GC, sub-millisecond, low-memory replacement for Java Lavalink.

---

## 🚀 Key Specifications

* **Binary Size**: `~8.4 MB` (stripped release with Link-Time Optimization).
* **RAM Footprint**: `~8 MB – 15 MB` (compared to 1.5 GB+ for Java Lavalink / JVM).
* **Startup Time**: `< 5 ms`.
* **Audio Engine**: `Songbird 0.4` + `Symphonia 0.5` (MP3, FLAC, AAC, OGG Vorbis, WAV).
* **Transport**: Direct 50 Hz UDP RTP Opus streaming with hardware-accelerated ChaCha20-Poly1305 and AES-GCM encryption.
* **Control Plane**: `Axum 0.8` + `Tokio` multi-threaded asynchronous runtime.
* **CPU Tuning**: Pre-configured worker thread throttling (`TOKIO_WORKER_THREADS=2`) optimized for 1-2 vCPU cloud servers (e.g. Oracle Cloud Free Tier, AWS t4g/t3a, Hetzner VPS).

---

## 🧩 Architecture & Extraction Subsystem

```
                         ┌────────────────────────────────────────────────────────┐
                         │                      axAudio Node                      │
                         │                                                        │
Discord Gateway ───────► │ 1. POST /v1/voice/join (Session + Token + Endpoint)   │
                         │ 2. Songbird Driver Handshake (Discord Voice UDP)       │
                         │                                                        │
Discord Bot Play ──────► │ 3. POST /v1/player/play (URL / Search Query)          │
                         │       │                                                │
                         │       ▼                                                │
                         │  Source Resolver (yt-dlp + Deno JS Runtime)            │
                         │       ├── Cookies (/app/cookies.txt)                   │
                         │       └── PoToken Sidecar (http://bgutil-pot:4416)     │
                         │       │                                                │
                         │       ▼                                                │
                         │  Direct Audio Stream (googlevideo CDN / HTTP / ICY)    │
                         │       │                                                │
                         │       ▼                                                │
                         │  Symphonia Audio Decoder & Opus Transcoder             │
                         │       │                                                │
                         │       ▼                                                │
                         │  Songbird TrackQueue ──► 50Hz Real-Time UDP RTP Engine │
                         └────────────────────────────────────────────────────────┘
```

---

## 🛡️ YouTube Anti-Bot & BotGuard Setup

Datacenter and cloud VPS IP addresses are routinely blocked by YouTube with `Sign in to confirm you’re not a bot`. `axAudio` comes with a built-in multi-layered bypass architecture:

1. **Deno JavaScript Engine**: YouTube encrypts player signatures using dynamic `base.js` scripts. `yt-dlp` requires a JavaScript engine to decode these signatures. `axAudio` bundles Deno v2 to fulfill this requirement natively.
2. **Proof-of-Origin (PoToken) Sidecar**: YouTube requires BotGuard PoTokens to authenticate requests. `axAudio` integrates with `brainicism/bgutil-ytdlp-pot-provider` to generate valid PoTokens on the fly without user intervention.
3. **Multi-Account Cookie Pool**: Support for multiple Google account cookies (directory `./cookies/*.txt` or `YTDLP_COOKIES_POOL`).
   * **Round-Robin Rotation**: Distributes video searches and streaming requests evenly across multiple accounts. This prevents YouTube from flagging or suspending an account for continuous search activity ("abnormal traffic").
   * **Automatic Retry**: If an account in the pool gets a temporary challenge or rate-limit, `axAudio` immediately retries with the next cookie in the pool.
   * **Cookie Lifespan**: Root credentials (`SID`, `HSID`, `SSID`) are valid for **1 to 2 years**. Rotating session tokens (`__Secure-1PSIDTS`, `SIDCC`) rotate frequently; `yt-dlp` updates them automatically in the file on every request if write permissions are enabled (`chmod 666`).
   * **Stream URL Timeout**: The ~6-hour expiration applies only to individual `googlevideo.com` CDN playback URLs, not your login cookies. `axAudio` resolves fresh URLs as tracks are requested and maintains an in-memory cache for fast replay.
4. **Studio-Grade Audio Fidelity & Bass Punch**:
   * **Native 48 kHz Opus Priority (Format 251)**: `axAudio` prioritizes YouTube's native 48,000 Hz Stereo Opus stream at ~160 kbps. This bypasses lossy AAC conversion, eliminates sample-rate conversion ringing (44.1k → 48k), and delivers uncompressed sub-bass (20 Hz - 100 Hz).
   * **Hardware-Accelerated Soft-Clipping**: Songbird's `use_softclip` applies a hyperbolic tangent (`tanh`) limiter curve to intersample peaks approaching 0 dBFS. This completely eliminates digital square-wave clipping (harsh screeching treble / "chillido") and retains deep, well-defined bass dynamics.
   * **Headroom Normalization**: Built-in 0.95 (-0.5 dBFS) true-peak margin prevents master track overs from compressing the audio stage.
   * **Configurable Opus Bitrate**: Up to 384 kbps (`AUDIO_BITRATE=128000`).

---

## 🐳 Quick Start: Running with Docker (Recommended)

The easiest and most reliable way to run `axAudio` in production or locally is via Docker Compose.

### Step 1: Export Your YouTube Cookies
1. Log into YouTube in a desktop browser (preferably using a secondary/throwaway Google account).
2. Use an extension such as **Get cookies.txt LOCALLY** (Chrome/Firefox) to export your cookies in Netscape format.
3. Save the exported file as `cookies.txt` in your project root directory.
4. **Important (Linux/VPS)**: Grant write permissions so `yt-dlp` can persist refreshed session tokens:
   ```bash
   chmod 666 cookies.txt
   ```

### Step 2: Start the Stack with Docker Compose
Run both `axAudio` and the `bgutil-pot` sidecar with a single command:

```bash
docker compose up -d
```

### Alternative: Run as a Standalone Docker Container
```bash
# 1. Start the PoToken provider sidecar on your docker network
docker run -d \
  --name bgutil-pot \
  --restart unless-stopped \
  -p 4416:4416 \
  brainicism/bgutil-ytdlp-pot-provider:latest

# 2. Build axAudio
docker build -t axaudio .

# 3. Run axAudio mounting your cookies.txt
docker run -d \
  --name axaudio \
  --restart unless-stopped \
  -p 2333:2333 \
  -e PORT=2333 \
  -e BGUTIL_POT_URL=http://host.docker.internal:4416 \
  -v $(pwd)/cookies.txt:/app/cookies.txt \
  axaudio
```

---

## 💻 Running Locally on Bare Metal

If you prefer compiling and running directly on your machine:

### Prerequisites

* **Rust & Cargo**: 1.80+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
* **Build Tools & Opus Library**:
  * **Ubuntu / Debian**: `sudo apt install -y cmake pkg-config libopus-dev python3 python3-pip`
  * **macOS**: `brew install cmake opus python`
  * **Windows**: Install [CMake](https://cmake.org/download/), [Python 3](https://www.python.org/), and ensure Opus libraries are in your `PATH` or use `vcpkg`.
* **Deno 2+**:
  * Linux / macOS: `curl -fsSL https://deno.land/install.sh | sh`
  * Windows: `irm https://deno.land/install.ps1 | iex`
* **Python Packages**:
  ```bash
  pip install -U yt-dlp bgutil-ytdlp-pot-provider
  ```

### Step 1: Run the PoToken Provider Sidecar
You can run the lightweight sidecar via Docker:
```bash
docker run -d --name bgutil-pot -p 4416:4416 brainicism/bgutil-ytdlp-pot-provider:latest
```

### Step 2: Provide Your `cookies.txt`
Place your exported Netscape `cookies.txt` in the root of the project directory.

### Step 3: Run axAudio
```bash
# Debug mode (fast compilation)
cargo run

# Or compile stripped release binary
cargo build --release
./target/release/axAudio
```

---

## ⚙️ Environment Variables

All settings can be customized through environment variables or a `.env` file:

| Variable | Default Value | Description |
| :--- | :--- | :--- |
| `PORT` | `2333` | HTTP server listening port. |
| `AUDIO_BITRATE` | `128000` | Voice channel Opus encoding bitrate in bps (e.g. 128000, 192000, 256000, 384000). |
| `RUST_LOG` | `axAudio=info,tower_http=info` | Tracing log level filter. |
| `TOKIO_WORKER_THREADS` | `2` | Number of Tokio worker threads (recommended: number of vCPUs). |
| `BGUTIL_POT_URL` | `http://bgutil-pot:4416` | URL to the Proof-of-Origin token provider sidecar. |
| `YTDLP_COOKIES_DIR` | `/app/cookies`, `./cookies` | Directory containing multiple `.txt` cookie files for automatic round-robin rotation. |
| `YTDLP_COOKIES_POOL` | _None_ | Comma-separated list of cookie file paths. |
| `YTDLP_COOKIES_PATH` | `/app/cookies.txt`, `./cookies.txt` | Path to single Netscape `cookies.txt` fallback file. |
| `YTDLP_COOKIES` | _None_ | Raw Netscape cookie content or path override. |
| `YTDLP_FORMAT` | `ba[format_id=251]/ba[acodec=opus]...` | Prioritizes YouTube's native 48kHz Opus stream (format 251) for high-definition audio. |
| `YTDLP_DEFAULT_SEARCH` | `ytsearch:` | Search engine provider prefix for text queries. |
| `YTDLP_REMOTE_COMPONENTS` | `ejs:github` | EJS remote solver source. Set to `none` to disable. |
| `YTDLP_EXTRACTOR_ARGS` | _Auto (bgutil)_ | Custom `--extractor-args` flags for `yt-dlp`. |
| `YTDLP_EXTRA_ARGS` | _None_ | Additional arbitrary flags forwarded directly to `yt-dlp`. |

---

## 📡 REST API Reference

All requests accept and return standard JSON payloads. Snowflake IDs (`guild_id`, `channel_id`, `user_id`) can be provided as numbers or numeric strings.

### 1. Health Check & Stats
```http
GET /health
GET /v1/stats
```

**Response**:
```json
{
  "success": true,
  "data": {
    "status": "healthy",
    "version": "0.1.0",
    "uptime_seconds": 120,
    "active_players": 1
  },
  "message": null
}
```

---

### 2. Connect to a Voice Channel
When Discord's Gateway dispatches `VOICE_STATE_UPDATE` and `VOICE_SERVER_UPDATE`, forward the session parameters to `axAudio`:

```http
POST /v1/voice/join
Content-Type: application/json

{
  "guild_id": "123456789012345678",
  "channel_id": "987654321098765432",
  "session_id": "session_id_from_discord",
  "token": "voice_token_from_discord",
  "endpoint": "us-east1234.discord.media",
  "user_id": "112233445566778899"
}
```

---

### 3. Enqueue / Play a Track
Enqueues a direct audio stream URL, a YouTube video URL, or a text search query.

```http
POST /v1/player/play
Content-Type: application/json

{
  "guild_id": "123456789012345678",
  "url": "https://www.youtube.com/watch?v=GitbDhOPBGs",
  "volume": 1.0,
  "extractor_args": null,
  "extra_args": null,
  "cookies": null
}
```

* `url`: Direct stream URL (`.mp3`, `.ogg`, etc.), YouTube URL, or search query (e.g. `ytsearch:lofi hip hop`).
* `volume` *(optional)*: Float multiplier from `0.0` to `2.0` (default: `1.0`).
* `extractor_args` *(optional)*: Per-request override for `yt-dlp --extractor-args`.
* `extra_args` *(optional)*: Per-request extra CLI flags for `yt-dlp`.
* `cookies` *(optional)*: Per-request Netscape cookies text or file path.
* `replace` *(optional)*: Boolean. If `true`, immediately stops the current stream and replaces it instead of appending to the queue.

*Supports YouTube Playlists (`/playlist?list=...`), Mixes (`&list=RD...`), and Radios! Automatically enqueues up to 25 tracks in sequence.*

---

### 4. Inspect Player Queue
Query the currently playing track and the upcoming queued tracks in Songbird's native queue:

```http
POST /v1/player/queue
Content-Type: application/json

{
  "guild_id": "123456789012345678"
}
```

**Response**:
```json
{
  "success": true,
  "data": {
    "guild_id": 123456789012345678,
    "is_playing": true,
    "current": {
      "title": "Song Title",
      "source_url": "https://www.youtube.com/watch?v=GitbDhOPBGs",
      "duration_secs": 214,
      "author": "Artist Name",
      "thumbnail": "https://i.ytimg.com/vi/GitbDhOPBGs/maxresdefault.jpg"
    },
    "queue": [
      {
        "title": "Next Song in Queue",
        "source_url": "https://www.youtube.com/watch?v=...",
        "duration_secs": 180,
        "author": "Another Artist",
        "thumbnail": "https://..."
      }
    ],
    "total_tracks": 2
  },
  "message": null
}
```

---

### 5. Player Control (Pause, Resume, Skip, Stop, Clear, Volume)
```http
POST /v1/player/control
Content-Type: application/json

{
  "guild_id": "123456789012345678",
  "action": "skip"
}
```

Supported `action` values:
* `"pause"`: Pauses current playback.
* `"resume"`: Resumes paused playback.
* `"skip"`: Skips the currently playing track to the next item in the queue.
* `"clear"`: Clears all upcoming tracks in the queue without interrupting the currently playing song.
* `"stop"`: Stops playback and clears the queue completely.
* `"volume"`: Adjusts output volume using the `value` field (`0.0` to `2.0`).
  ```json
  {
    "guild_id": "123456789012345678",
    "action": "volume",
    "value": 0.5
  }
  ```

---

### 5. Resolve Track Metadata Without Playing
Query track metadata and extract the direct CDN stream URL without playing it:

```http
POST /v1/tracks/resolve
Content-Type: application/json

{
  "query": "https://www.youtube.com/watch?v=GitbDhOPBGs"
}
```

**Response**:
```json
{
  "success": true,
  "data": {
    "title": "Song Title",
    "source_url": "https://www.youtube.com/watch?v=GitbDhOPBGs",
    "stream_url": "https://rr3---sn-...googlevideo.com/videoplayback?...",
    "duration_secs": 214,
    "author": "Artist Name",
    "thumbnail": "https://i.ytimg.com/vi/GitbDhOPBGs/maxresdefault.jpg"
  },
  "message": null
}
```

---

### 6. Disconnect from Voice Channel
```http
POST /v1/voice/leave
Content-Type: application/json

{
  "guild_id": "123456789012345678"
}
```

---

## 🤖 Discord Bot Integration Example (TypeScript / Discord.js)

```typescript
import axios from 'axios';

const AXAUDIO_URL = 'http://127.0.0.1:2333';

// 1. Join Voice Channel
async function joinVoice(guild, channel, botUser) {
  // Collect Discord Gateway Voice Update events:
  // - GatewayDispatchEvents.VoiceServerUpdate
  // - GatewayDispatchEvents.VoiceStateUpdate
  const voiceData = await waitForDiscordVoiceHandshake(guild.id);

  await axios.post(`${AXAUDIO_URL}/v1/voice/join`, {
    guild_id: guild.id,
    channel_id: channel.id,
    session_id: voiceData.sessionId,
    token: voiceData.token,
    endpoint: voiceData.endpoint,
    user_id: botUser.id,
  });
}

// 2. Play Audio
async function playTrack(guildId: string, query: string) {
  const response = await axios.post(`${AXAUDIO_URL}/v1/player/play`, {
    guild_id: guildId,
    url: query,
    volume: 1.0,
  });
  return response.data;
}

// 3. Skip Track
async function skipTrack(guildId: string) {
  await axios.post(`${AXAUDIO_URL}/v1/player/control`, {
    guild_id: guildId,
    action: 'skip',
  });
}
```

---

## 📜 License

Licensed under the MIT License.
