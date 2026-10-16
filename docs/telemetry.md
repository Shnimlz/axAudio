# Real-Time WebSocket Telemetry

axAudio exposes a high-throughput, low-latency WebSocket event stream on `GET /v1/events`. This enables client Discord bots to track track lifecycle transitions (`track_start`, `track_end`, `track_error`) without polling the REST API.

---

## 1. Architectural Model

```
 ┌──────────────────────┐        ┌──────────────────────┐
 │ Songbird Track Event │ ─────► │ Tokio Broadcast Bus  │
 │ (Play / End / Error) │        │ (event_tx: capacity) │
 └──────────────────────┘        └──────────┬───────────┘
                                            │
                                            ▼
                      ┌───────────────────────────────────────────┐
                      │ Axum WebSocket Streamer (/v1/events)      │
                      │ • Per-client broadcast subscriber         │
                      │ • Automated Ping / Pong handling          │
                      │ • Lag detection and backpressure drop     │
                      └─────────────────────┬─────────────────────┘
                                            │ (JSON Frames)
                                            ▼
                               [Discord Bot Client WS]
```

### Event Bus Guarantees
- **Non-blocking Dispatch**: Real-time audio threads never wait on WebSocket network I/O. Events are pushed to an asynchronous `tokio::sync::broadcast` channel.
- **Client Isolation**: Slow or stalled WebSocket consumers do not degrade audio transmission or block other clients. If a subscriber falls behind the broadcast ring buffer, axAudio logs a lag warning and advances the client cleanly:
  ```rust
  Err(broadcast::error::RecvError::Lagged(missed)) => {
      warn!("WebSocket client lagged behind, skipped {} events", missed);
  }
  ```

---

## 2. Connecting to the Stream

Connect using standard WebSocket clients:

```
ws://<axaudio-host>:2333/v1/events
```

No authentication headers are required on the WebSocket upgrade request.

---

## 3. Event Packet Specifications

All events use a tagged JSON structure:

```typescript
type PlayerEvent = TrackStartEvent | TrackEndEvent | TrackErrorEvent;
```

---

### `track_start`
Fired immediately when Songbird begins outputting the first audio packet of a track to Discord voice servers.

```json
{
  "type": "track_start",
  "guild_id": 1198516089758785536,
  "track": {
    "title": "Starboy",
    "source_url": "https://www.youtube.com/watch?v=34Na4j8AVgA",
    "stream_url": "https://rr1---sn-4g5edn6e.googlevideo.com/videoplayback?...",
    "duration_secs": 230,
    "author": "The Weeknd",
    "thumbnail": "https://i.ytimg.com/vi/34Na4j8AVgA/hqdefault.jpg"
  }
}
```

Use this event to:
- Update "Now Playing" embeds.
- Update rich presence activities in Discord Activities.
- Trigger equalizer presets specific to genres.

---

### `track_end`
Fired when a track finishes playback naturally or when skipped by a user.

```json
{
  "type": "track_end",
  "guild_id": 1198516089758785536,
  "track": {
    "title": "Starboy",
    "source_url": "https://www.youtube.com/watch?v=34Na4j8AVgA",
    "stream_url": "https://rr1---sn-4g5edn6e.googlevideo.com/videoplayback?...",
    "duration_secs": 230,
    "author": "The Weeknd",
    "thumbnail": "https://i.ytimg.com/vi/34Na4j8AVgA/hqdefault.jpg"
  },
  "reason": "finished"
}
```

Use this event to:
- Clean up player messages.
- Fetch and display the next song in queue.

---

### `track_error`
Fired when a track experiences a decode failure, CDN stream drop, or socket timeout during playback.

```json
{
  "type": "track_error",
  "guild_id": 1198516089758785536,
  "track": {
    "title": "Corrupted Audio File",
    "source_url": "https://example.com/audio.mp3",
    "stream_url": "https://example.com/audio.mp3",
    "duration_secs": 120,
    "author": "Unknown",
    "thumbnail": null
  },
  "error": "InputError: Connection closed before full payload was decoded"
}
```

Use this event to:
- Notify Discord text channels of streaming failures.
- Auto-skip broken tracks in unattended queues.

---

## 4. Connection Resilience & Ping / Pong

axAudio handles WebSocket protocol `Ping` frames natively, returning an immediate `Pong` frame with matching payload.

### Recommended Client Heartbeat:
Client applications should send periodic Ping frames or application-level keep-alives every 15–30 seconds.

### Reconnection Strategy:
If the WebSocket drops:
1. Reconnect with exponential backoff (e.g. 1s, 2s, 4s, up to 10s max).
2. Upon reconnecting, issue a `POST /v1/player/queue` request for each active guild to resynchronize the UI state.

---

## 5. Client Integration Example (TypeScript / Node.js)

```typescript
import WebSocket from "ws";

interface Track {
  title: string;
  source_url: string;
  duration_secs?: number;
  author?: string;
  thumbnail?: string;
}

interface TelemetryEvent {
  type: "track_start" | "track_end" | "track_error";
  guild_id: number | string;
  track?: Track;
  reason?: string;
  error?: string;
}

export class AxAudioTelemetry {
  private ws: WebSocket | null = null;
  private url: string;

  constructor(host = "127.0.0.1:2333") {
    this.url = `ws://${host}/v1/events`;
    this.connect();
  }

  private connect() {
    this.ws = new WebSocket(this.url);

    this.ws.on("open", () => {
      console.log("Connected to axAudio telemetry bus");
    });

    this.ws.on("message", (data: WebSocket.Data) => {
      try {
        const event: TelemetryEvent = JSON.parse(data.toString());
        this.handleEvent(event);
      } catch (err) {
        console.error("Failed to parse telemetry event:", err);
      }
    });

    this.ws.on("close", () => {
      console.warn("Telemetry socket closed, reconnecting in 3s...");
      setTimeout(() => this.connect(), 3000);
    });

    this.ws.on("error", (err) => {
      console.error("Telemetry socket error:", err);
    });
  }

  private handleEvent(event: TelemetryEvent) {
    const guildId = String(event.guild_id);

    switch (event.type) {
      case "track_start":
        console.log(`[Guild ${guildId}] Now playing: ${event.track?.title}`);
        break;

      case "track_end":
        console.log(`[Guild ${guildId}] Track ended: ${event.track?.title} (${event.reason})`);
        break;

      case "track_error":
        console.error(`[Guild ${guildId}] Error playing ${event.track?.title}: ${event.error}`);
        break;
    }
  }
}
```
