# ⚡ axAudio

**Ultra-lightweight, Bare-Metal Discord Native Audio Engine in Rust.**  
Designed as a zero-GC, sub-millisecond replacement for Java Lavalink.

---

## 🚀 Key Specifications

* **Binary Size**: `~8.4 MB` (stripped release with Link-Time Optimization).
* **RAM Footprint**: `~8 MB – 15 MB` (compared to 1.5 GB+ for Java Lavalink).
* **Startup Time**: `< 5 ms`.
* **Audio Engine**: `Songbird 0.4` + `Symphonia 0.5` (MP3, FLAC, AAC, OGG Vorbis).
* **Transport**: Direct 50Hz UDP RTP streaming with hardware-accelerated ChaCha20-Poly1305 and AES-GCM encryption.
* **Control Plane**: Axum 0.8 + Tokio multi-threaded asynchronous runtime.

---

## 📡 API Endpoints (Port 2333)

### 1. Health & Telemetry
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

### 2. Connect to Voice Channel
When Discord's Gateway dispatches `VOICE_STATE_UPDATE` and `VOICE_SERVER_UPDATE`, forward the credentials:

```http
POST /v1/voice/join
Content-Type: application/json

{
  "guild_id": 123456789012345678,
  "channel_id": 987654321098765432,
  "session_id": "session_string_from_discord",
  "token": "voice_token_from_discord",
  "endpoint": "us-east1234.discord.media",
  "user_id": 112233445566778899
}
```

---

### 3. Play Audio Stream
Stream any direct HTTP audio stream (MP3, OGG, AAC, WebM, etc.):

```http
POST /v1/player/play
Content-Type: application/json

{
  "guild_id": 123456789012345678,
  "url": "https://stream.example.com/audio.mp3",
  "volume": 0.8
}
```

---

### 4. Player Controls
```http
POST /v1/player/control
Content-Type: application/json

{
  "guild_id": 123456789012345678,
  "action": "pause" // "pause" | "resume" | "stop" | "skip" | "volume",
  "value": 1.2     // optional, used when action is "volume" (0.0 to 2.0)
}
```

---

### 5. Disconnect & Cleanup
```http
POST /v1/voice/leave
Content-Type: application/json

{
  "guild_id": 123456789012345678
}
```

---

## 🛠️ Build & Run

### Development
```bash
cargo run
```

### Production Release (8.4 MB)
```bash
cargo build --release
./target/release/axAudio
```
