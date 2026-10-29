# REST API Reference

axAudio provides a streamlined, type-safe REST control plane built on Axum 0.8. All endpoints consume and produce UTF-8 `application/json`.

---

## 1. Response Envelope

All API endpoints return responses encapsulated in a standardized envelope:

```typescript
interface ApiResponse<T> {
  success: boolean;
  data: T | null;
  message?: string;
}
```

- **Success**: `success: true`, `data` contains the payload, `message` is omitted or contains a human-readable confirmation.
- **Error**: `success: false`, `data: null`, `message` provides the exact error explanation. HTTP status codes follow standard REST semantics (`400 Bad Request`, `404 Not Found`, `500 Internal Error`).

---

## 2. Snowflake ID Flexibility & Multi-Bot Isolation

Discord IDs (`guild_id`, `channel_id`, `user_id`, `bot_id`) exceed JavaScript's `Number.MAX_SAFE_INTEGER` ($2^{53} - 1$). axAudio handles IDs seamlessly as either **numeric strings** or **raw 64-bit unsigned integers**:

```json
{ "guild_id": "1198516089758785536", "bot_id": "100234857289384729" }
// or
{ "guild_id": 1198516089758785536, "bot_id": 100234857289384729 }
```

### Multi-Bot Isolation
To run multiple Discord bots on a single axAudio server instance without session collisions or hijacking, pass `bot_id` (also accepts aliases `user_id` or `bot`) in any player/voice request. axAudio isolates voice drivers and player state by `(guild_id, bot_id)`. If omitted, axAudio automatically binds to the active player in that guild or defaults to single-bot compatibility.

---

## 3. Node & Diagnostics Endpoints

### `GET /health`
Liveness check for container orchestrators and reverse proxies.

**Response `200 OK`**:
```json
{
  "success": true,
  "data": "axAudio engine operational",
  "message": null
}
```

---

### `GET /v1/stats`
Retrieves engine telemetry, versioning, uptime, player counts, memory consumption, and resolution statistics.

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "status": "healthy",
    "version": "0.1.0",
    "uptime_seconds": 18420,
    "active_players": 3,
    "active_247_players": 1,
    "tracks_played_total": 42,
    "resolves_total": 128,
    "cached_tracks": 15,
    "custom_presets_count": 2,
    "memory_rss_bytes": 14680064
  },
  "message": null
}
```

---

### `GET /metrics`
Standard Prometheus exposition format (`text/plain; version=0.0.4`) for direct scraping by Prometheus, Grafana Agent, Datadog, or VictoriaMetrics without authentication requirements.

**Response `200 OK`**:
```prometheus
# HELP axaudio_uptime_seconds Uptime of the axAudio engine in seconds
# TYPE axaudio_uptime_seconds counter
axaudio_uptime_seconds 18420

# HELP axaudio_active_players Number of currently active Discord voice players
# TYPE axaudio_active_players gauge
axaudio_active_players 3

# HELP axaudio_247_players Number of players with 24/7 mode enabled
# TYPE axaudio_247_players gauge
axaudio_247_players 1

# HELP axaudio_tracks_played_total Total number of audio tracks played since startup
# TYPE axaudio_tracks_played_total counter
axaudio_tracks_played_total 42

# HELP axaudio_resolves_total Total number of track resolution queries executed
# TYPE axaudio_resolves_total counter
axaudio_resolves_total 128

# HELP axaudio_cached_tracks Number of tracks currently in memory cache
# TYPE axaudio_cached_tracks gauge
axaudio_cached_tracks 15

# HELP axaudio_custom_presets Number of custom user-defined presets registered
# TYPE axaudio_custom_presets gauge
axaudio_custom_presets 2

# HELP axaudio_memory_rss_bytes Process resident set size (RSS) memory in bytes
# TYPE axaudio_memory_rss_bytes gauge
axaudio_memory_rss_bytes 14680064
```

---

## 4. Voice Channel Management

### `POST /v1/voice/join`
Connects an axAudio driver instance to a Discord voice channel using the credentials supplied by Discord's Voice State Update and Voice Server Update gateway events.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "channel_id": "1198516089758785540",
  "session_id": "9a38f3238ab823c9",
  "token": "49bf32890ca...",
  "endpoint": "us-east123.discord.media:443",
  "user_id": "100234857289384729"
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": "Connected to Discord voice channel",
  "message": null
}
```

---

### `POST /v1/voice/leave`
Disconnects the voice driver, cancels any active background playlist workers, and cleans up guild state.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536"
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": "Disconnected and cleaned up player",
  "message": null
}
```

---

## 5. Playback & Queue Management

### `POST /v1/player/play`
Enqueues a song, playlist, mix, or direct audio stream.

#### Fast-Start Architecture:
When a playlist or YouTube Mix is submitted, axAudio immediately resolves and streams Track 1 into the voice channel. A background worker then resolves the remaining tracks asynchronously, with CPU cooldown pauses, ensuring sub-second response times regardless of playlist length.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
  "volume": 1.0,
  "replace": false,
  "extractor_args": "youtube:player-client=web_creator",
  "cookies": "/app/cookies.txt"
}
```

| Field | Type | Description |
| :--- | :--- | :--- |
| `guild_id` | `u64` / `string` | Guild snowflake ID. |
| `url` | `string` | Track URL, search query, playlist URL, or direct audio link (`.mp3`, `.ogg`, ICY radio). |
| `volume` | `float` (optional) | Initial volume scale (`0.0` to `2.0`, default: `1.0`). |
| `replace` | `bool` (optional) | If `true`, halts current playback, cancels pending background playlist jobs, and replaces the queue. |
| `extractor_args` | `string` (optional) | Custom extractor arguments forwarded to `yt-dlp`. |
| `cookies` | `string` (optional) | Explicit cookie path overriding the automatic pool. |
| `format` | `string` (optional) | Custom format selection (default: best audio format). |

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "title": "Never Gonna Give You Up",
    "source_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    "stream_url": "https://rr3---sn-4g5edn6e.googlevideo.com/videoplayback?...",
    "duration_secs": 213,
    "author": "Rick Astley",
    "thumbnail": "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
    "is_playlist": false,
    "playlist_title": null,
    "playlist_tracks_count": 1,
    "queue_length": 1,
    "was_empty": true
  },
  "message": null
}
```

---

### `POST /v1/player/live`
Dedicated endpoint for continuous internet radio streaming (Icecast, SHOUTcast, continuous HTTP streams). Features an adaptive buffer and an automatic ICY metadata demuxer that strips interleaved metadata before decoding (preventing Symphonia frame desync errors) and broadcasts real-time `metadata_update` WebSocket events when track titles change on the station. Full DSP processing (EQ, Spatial 3D, Reverb, Loudness) applies identically to live radio streams.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "url": "https://stream.radioparadise.com/flac",
  "title": "Radio Paradise (Main Mix)",
  "volume": 0.85
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "title": "Radio Paradise (Main Mix)",
    "source_url": "https://stream.radioparadise.com/flac",
    "stream_url": "https://stream.radioparadise.com/flac",
    "duration_secs": null,
    "author": "Live Radio",
    "thumbnail": null,
    "is_playlist": false,
    "playlist_title": null,
    "playlist_tracks_count": 0,
    "queue_length": 1,
    "was_empty": true
  },
  "message": null
}
```

---

### `POST /v1/player/control`
Executes an immediate playback action on the guild's voice driver.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "action": "pause",
  "value": null
}
```

| Action | Description | `value` Requirement |
| :--- | :--- | :--- |
| `"pause"` | Suspends audio playback. | None. |
| `"resume"` | Resumes audio playback. | None. |
| `"stop"` | Stops playback, terminates active streams, cancels background playlist workers. | None. |
| `"skip"` | Advances to the next track in queue immediately. | None. |
| `"clear"` | Clears the upcoming queue while preserving the active track. | None. |
| `"volume"` | Adjusts the active track volume (`0.0` to `2.0`). | Required `float`. |

**Response `200 OK`**:
```json
{
  "success": true,
  "data": "Player paused",
  "message": null
}
```

---

### `POST /v1/player/247`
Toggles or explicitly sets 24/7 mode for a player. In 24/7 mode, the voice driver is never disconnected by the idle Garbage Collector even when the queue remains empty indefinitely.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "enabled": true
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": "Modo 24/7 activado",
  "message": null
}
```

---

### `POST /v1/player/queue`
Inspects the guild's current playback queue, including tracks currently being fetched by background playlist workers.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536"
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "guild_id": 1198516089758785536,
    "is_playing": true,
    "current": {
      "title": "Starboy",
      "source_url": "https://www.youtube.com/watch?v=34Na4j8AVgA",
      "stream_url": "https://rr1---sn-...",
      "duration_secs": 230,
      "author": "The Weeknd",
      "thumbnail": "https://i.ytimg.com/vi/34Na4j8AVgA/hqdefault.jpg"
    },
    "queue": [
      {
        "title": "Blinding Lights",
        "source_url": "https://www.youtube.com/watch?v=4NRXx6U8ABQ",
        "stream_url": "https://rr2---sn-...",
        "duration_secs": 200,
        "author": "The Weeknd",
        "thumbnail": "https://i.ytimg.com/vi/4NRXx6U8ABQ/hqdefault.jpg"
      },
      {
        "title": "⏳ Save Your Tears",
        "source_url": "https://www.youtube.com/watch?v=XXYlFuWEuKI",
        "stream_url": "",
        "duration_secs": 215,
        "author": "The Weeknd",
        "thumbnail": null
      }
    ],
    "total_tracks": 3
  },
  "message": null
}
```
*(Notice: pending background playlist items are prefixed with `⏳` until resolved).*

---

## 6. DSP & Equalization Endpoints

### `POST /v1/player/eq`
Configures the multi-mode studio equalizer or applies an acoustic preset.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "preset": "moto-spatial",
  "bands": [
    { "band": 0, "gain": 4.5, "filter_type": "peaking" },
    { "band": 1, "gain": 3.0, "filter_type": "lowshelf" }
  ],
  "parametric": [
    { "freq": 1000.0, "q": 1.4, "gain_db": 3.0, "filter_type": "peaking", "channel": "Mid" },
    { "freq": 12000.0, "q": 0.7, "gain_db": 4.5, "filter_type": "highshelf", "channel": "Side" },
    { "freq": 60.0, "q": 12.0, "gain_db": -24.0, "filter_type": "notch", "channel": "Stereo" }
  ],
  "dolby_bands": [3.0, 2.5, 2.0, 1.5, 1.0, 0.5, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.0, 3.5, 3.0, 2.5],
  "iso31_bands": [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 2.5, 2.0, 1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 4.0, 3.5, 3.0, 2.5, 2.0, 1.5, 1.0]
}
```

#### Field Specifications:
- `preset` (optional): Name of a factory preset or custom preset (18 built-in presets supported).
- `bands` (optional): Array of 15-band graphic equalizer adjustments `{ band: 0..14, gain: float, filter_type?: string }`.
  - Values between `-0.5` and `1.0` are converted automatically using the Lavalink scale; values outside this range are interpreted directly as decibels (`-15.0 dB` to `+15.0 dB`).
- `parametric` (optional): Array of dynamic studio filter bands `{ freq: float, q: float, gain_db: float, filter_type?: string, channel?: string }`.
  - `freq`: 20.0 Hz to 22,000.0 Hz.
  - `q`: 0.1 to 20.0.
  - `gain_db`: -24.0 dB to +24.0 dB.
  - `filter_type`: `"peaking"`, `"lowshelf"`, `"highshelf"`, `"lowpass"`, `"highpass"`, `"notch"`, `"bandpass"`.
  - `channel`: `"Stereo"` (default), `"Mid"` (mono center: vocals, kick, snare, bass), or `"Side"` (stereo difference: air, room reverb, wide synths).
- `dolby_bands` (optional): Array of exactly 20 decibel gains matching the psychoacoustic critical bands of Dolby DAX:
  `[47, 141, 234, 328, 422, 562, 750, 984, 1266, 1641, 2109, 2719, 3469, 4406, 5578, 7078, 9000, 11484, 14766, 19688] Hz`.
- `iso31_bands` (optional): Array of exactly 31 decibel gains matching the ISO 1/3-octave studio frequencies (20 Hz to 20 kHz).

**Supported Filter Types**:
- `"peaking"` (default): Bell curve centered at the band frequency.
- `"lowshelf"`: Boosts or cuts all frequencies below the band frequency.
- `"highshelf"`: Boosts or cuts all frequencies above the band frequency.
- `"lowpass"`: Butterworth 2nd-order low-pass filter (attenuates high frequencies).
- `"highpass"`: Butterworth 2nd-order high-pass filter (attenuates low frequencies).
- `"notch"`: Narrow band-reject filter for hum or feedback elimination.
- `"bandpass"`: Bandpass filter with 0 dB peak gain.

**Supported Built-In Presets (18 profiles)**:
- **Motorola Dolby Atmos**: `moto-spatial` (calibrated for moto buds bass), `dolby-music`, `dolby-movie`, `dolby-game`, `dolby-voice`.
- **Studio Audiophile**: `music` (Hi-Fi), `cinema-3d` (Atmos immersion), `bass-boost-3d`, `vocal-clarity`, `gaming-3d`, `concert-hall`.
- **Effects & Loudness**: `nightcore`, `vaporwave`, `8d`, `tremolo`, `vibrato`, `broadcast` (EBU R128), `flat` (bypass).

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "guild_id": 1198516089758785536,
    "status": {
      "active_preset": "moto-spatial",
      "stereo_width": 1.25,
      "binaural_crossfeed": true,
      "bass_exciter": true,
      "eq_bands": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    },
    "message": "Preset 'moto-spatial' aplicado: Motorola Dolby Atmos Spatial Audio calibrado vía ADB (moto buds bass)..."
  }
}
```

---

### `POST /v1/player/dsp`
Full control over the DSP mastering chain, including multi-mode EQ, Mid/Side stereo widening, acoustic crossfeed, and the sub-bass exciter.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "preset": "moto-spatial",
  "spatial": {
    "stereo_width": 1.35,
    "binaural_crossfeed": true,
    "crossfeed_level": 0.18,
    "bass_exciter": true,
    "bass_exciter_drive": 0.25
  },
  "parametric": [
    { "freq": 1000.0, "q": 1.4, "gain_db": 3.0, "channel": "Mid" }
  ],
  "dolby_bands": null,
  "iso31_bands": null,
  "bands": null
}
```

| Field | Range | Default | Purpose |
| :--- | :--- | :--- | :--- |
| `stereo_width` | `0.0` (Mono) to `2.0` (Ultra Wide) | `1.0` | Phase-coherent Mid/Side stereo widening. |
| `binaural_crossfeed` | `boolean` | `false` | Enables head-shadow diffraction filter. |
| `crossfeed_level` | `0.0` to `0.40` | `0.18` | Blending coefficient of cross-ear leakage. |
| `bass_exciter` | `boolean` | `false` | Enables sub-85 Hz harmonic exciter. |
| `bass_exciter_drive` | `0.0` to `1.0` | `0.35` | Saturation intensity of generated harmonics. |

---

### Granular DSP Endpoints

Clients can configure specific DSP stages independently without sending full pipeline payloads:

#### `POST /v1/player/dsp/reverb`
Configures the 8-node Feedback Delay Network (FDN) acoustic reverberation room simulation.
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "active": true,
  "rt60": 2.5,
  "wet": 0.35,
  "dry": 1.0,
  "tone": 0.6,
  "pre_delay_ms": 25.0
}
```

#### `POST /v1/player/dsp/spatial`
Configures Mid/Side widening, binaural crossfeed, and harmonic sub-bass exciter.
```json
{
  "guild_id": "1198516089758785536",
  "stereo_width": 1.4,
  "binaural_crossfeed": true,
  "crossfeed_level": 0.2,
  "bass_exciter": true,
  "bass_exciter_drive": 0.3
}
```

#### `POST /v1/player/dsp/timescale`
Independent control over playback speed, musical pitch, and rate with zero chipmunk effect (WSOLA + Hermite spline).
```json
{
  "guild_id": "1198516089758785536",
  "speed": 1.25,
  "pitch": 1.0,
  "rate": 1.0
}
```

#### `POST /v1/player/dsp/loudness`
ITU-R BS.1770-4 / EBU R128 real-time loudness normalizer with True-Peak brickwall limiter.
```json
{
  "guild_id": "1198516089758785536",
  "target_lufs": -14.0,
  "gate_threshold": -70.0,
  "attack_time": 0.05,
  "release_time": 0.5,
  "limiter_active": true
}
```

#### `POST /v1/player/dsp/modulation`
LFO modulation effects: 8D circular rotation, Tremolo, Vibrato, Chorus, Flanger, and Phaser.
```json
{
  "guild_id": "1198516089758785536",
  "rotation_8d": true,
  "rotation_speed": 0.2,
  "chorus": true,
  "chorus_depth": 0.35,
  "chorus_rate": 1.2
}
```

#### `POST /v1/player/dsp/reset`
Instantly resets the entire DSP mastering pipeline for the player back to flat bypass.
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729"
}
```

---

### `POST /v1/presets`
Saves current guild DSP settings (or default settings) as a named custom preset with automatic persistent disk storage (`presets/custom_presets.json`).

**Request Body**:
```json
{
  "name": "my_bass_boost",
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "description": "Deep club sub-bass with stereo widening"
}
```

---

### `GET /v1/presets/list`
Lists all user-defined custom presets saved in the system.

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "presets": [
      {
        "name": "my_bass_boost",
        "description": "Deep club sub-bass with stereo widening",
        "status": {
          "stereo_width": 1.4,
          "binaural_crossfeed": true,
          "eq_bands": [6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0]
        }
      }
    ],
    "count": 1
  },
  "message": null
}
```

---

### `POST /v1/presets/apply`
Applies a custom or built-in preset to a specific guild player.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "bot_id": "100234857289384729",
  "preset": "my_bass_boost"
}
```

---

### `POST /v1/presets/delete`
Deletes a custom preset from memory and disk.

**Request Body**:
```json
{
  "name": "my_bass_boost"
}
```

---

## 7. Metadata & Discovery Endpoints

### `POST /v1/tracks/resolve`
Resolves metadata and stream URL for a given track without enqueuing it.

**Request Body**:
```json
{
  "query": "https://www.youtube.com/watch?v=kJQP7kiw5Fk"
}
```

---

### `POST /v1/tracks/search`
Searches YouTube or SoundCloud for tracks matching a keyword query.

**Request Body**:
```json
{
  "query": "Daft Punk Get Lucky",
  "limit": 5,
  "source": "youtube"
}
```

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "query": "Daft Punk Get Lucky",
    "source_used": "youtube",
    "tracks": [
      {
        "title": "Daft Punk - Get Lucky (Official Audio) ft. Pharrell Williams",
        "url": "https://www.youtube.com/watch?v=5NV6Rdv1a3I",
        "duration_secs": 248,
        "author": "Daft Punk",
        "thumbnail": "https://i.ytimg.com/vi/5NV6Rdv1a3I/hqdefault.jpg"
      }
    ]
  },
  "message": null
}
```
