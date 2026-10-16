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

## 2. Snowflake ID Flexibility

Discord IDs (`guild_id`, `channel_id`, `user_id`) exceed JavaScript's `Number.MAX_SAFE_INTEGER` ($2^{53} - 1$). axAudio handles IDs seamlessly as either **numeric strings** or **raw 64-bit unsigned integers**:

```json
{ "guild_id": "1198516089758785536" }
// or
{ "guild_id": 1198516089758785536 }
```

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
Retrieves engine telemetry, versioning, uptime, and active player counts.

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "status": "healthy",
    "version": "0.1.0",
    "uptime_seconds": 18420,
    "active_players": 3
  },
  "message": null
}
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
Configures the 15-band graphic equalizer or applies an acoustic preset.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "preset": "cinema-3d",
  "bands": [
    { "band": 0, "gain": 4.5 },
    { "band": 1, "gain": 3.0 }
  ]
}
```

- `preset`: Name of a factory preset (`music`, `cinema-3d`, `bass-boost-3d`, `vocal-clarity`, `gaming-3d`, `concert-hall`, `flat`).
- `bands`: Array of `{ band: 0..14, gain: float }`. Values between `-0.5` and `1.0` are converted automatically using the Lavalink scale; values outside this range are interpreted directly as decibels (`-15.0 dB` to `+15.0 dB`).

**Response `200 OK`**:
```json
{
  "success": true,
  "data": {
    "guild_id": 1198516089758785536,
    "status": {
      "active_preset": "cinema-3d",
      "stereo_width": 1.35,
      "binaural_crossfeed": true,
      "bass_exciter": true,
      "eq_bands": [3.5, 3.0, 2.0, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0]
    },
    "message": "Preset 'cinema-3d' aplicado: Inmersión cinemática 3D tipo Dolby Atmos..."
  }
}
```

---

### `POST /v1/player/dsp`
Full control over the DSP mastering chain, including Mid/Side stereo widening, acoustic crossfeed, and the sub-bass exciter.

**Request Body**:
```json
{
  "guild_id": "1198516089758785536",
  "preset": null,
  "spatial": {
    "stereo_width": 1.35,
    "binaural_crossfeed": true,
    "crossfeed_level": 0.18,
    "bass_exciter": true,
    "bass_exciter_drive": 0.25
  },
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
