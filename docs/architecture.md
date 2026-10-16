# Architecture & Concurrency Model

This document outlines the architectural foundation, threading model, memory strategy, and audio delivery guarantees of **axAudio**.

---

## 1. Design Motivation

Traditional Discord voice server implementations rely on Java-based runtimes like Lavalink. While feature-rich, the Java Virtual Machine incurs considerable baseline overhead:

- **Baseline RAM footprint**: 500 MB to 1.5 GB resident memory, caused by JVM class loading, object allocations, and generational garbage collection metadata.
- **Garbage Collection (GC) Jitter**: Stop-the-world or incremental GC pauses can disrupt the strict 20ms audio frame transmission cadence, resulting in audible stutter, packet drops, or Discord gateway disconnections.
- **Startup Latency**: JVM bytecode verification and JIT warm-up take multiple seconds before the node can service its first HTTP request.

axAudio was engineered from the ground up in modern systems Rust to provide a deterministic, zero-GC, ultra-lightweight alternative.

```
┌────────────────────────────────────────────────────────────────────────┐
│                              axAudio Node                              │
│                                                                        │
│  ┌──────────────────────┐               ┌───────────────────────────┐  │
│  │   Axum HTTP Engine   │               │     Discord Voice Driver  │  │
│  │   (Control Plane)    │               │       (Songbird 0.4)      │  │
│  │                      │               │                           │  │
│  │  • Voice Handshake   │               │  • 50 Hz RTP Scheduler    │  │
│  │  • Play / Stop / Skip│               │  • ChaCha20 / AES-GCM     │  │
│  │  • Realtime 15-EQ    │               │  • UDP Voice Socket       │  │
│  └──────────┬───────────┘               └─────────────▲─────────────┘  │
│             │                                         │                │
│             ▼                                         │                │
│  ┌──────────────────────┐               ┌─────────────┴─────────────┐  │
│  │   Source Resolver    │ ── Stream ──► │     Planar DSP Chain      │  │
│  │ (yt-dlp / PoToken)   │    URL        │  (TDF-II Biquads + Spatial)│  │
│  └──────────────────────┘               └───────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Memory Model & Zero-GC Guarantees

In real-time audio programming, dynamic memory allocation inside the audio thread is a primary source of buffer under-runs. axAudio enforces strict allocation constraints:

1. **Pre-allocated Buffers**: The audio pipeline allocates intermediate scratch buffers (`AudioBuffer<f32>`) once per track or format transition. These buffers persist across decoding calls and are resized only if packet capacity changes.
2. **In-Place Planar Processing**: Audio samples are processed as planar slices:
   ```rust
   pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32])
   ```
   Filter states mutate contiguous slices in-place without copying audio buffers.
3. **No Garbage Collection**: Memory is reclaimed immediately when a track completes or a guild disconnects using standard RAII ownership semantics.
4. **Resident Footprint**: Baseline memory consumption is **8–15 MB RAM**, expanding by only ~2 MB per concurrent active voice stream.

---

## 3. Threading Architecture

axAudio runs on a multi-threaded Tokio runtime tuned specifically for low-core server environments (such as free-tier VPS instances with 1 to 2 vCPUs).

```
                      TOKIO RUNTIME THREAD POOL
  ┌───────────────────────────────────────────────────────────────┐
  │  Worker Thread 0              │  Worker Thread 1              │
  │  • Axum HTTP Server           │  • Axum HTTP Server           │
  │  • WebSocket Telemetry Bus    │  • Fast Track Enqueueing      │
  │  • Tokio Channel Dispatch     │  • State Coordination         │
  └───────────────────────────────┴───────────────────────────────┘
                                  │
                                  ▼
                     BLOCKING THREAD POOL (Max 4)
  ┌───────────────────────────────────────────────────────────────┐
  │  • yt-dlp Subprocess Execution (Bounded by Semaphore = 1)     │
  │  • DNS Resolution and Initial HTTPS Stream Handshake          │
  └───────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
                     SONGBIRD REAL-TIME VOICE LOOP
  ┌───────────────────────────────────────────────────────────────┐
  │  • Dedicated 50 Hz Voice Driver Thread                        │
  │  • Reads PCM packets every 20ms                               │
  │  • Opus compression + ChaCha20-Poly1305 packet encryption     │
  │  • Direct UDP socket write to Discord media gateway           │
  └───────────────────────────────────────────────────────────────┘
```

### Runtime Configuration

In `src/main.rs`, the runtime is configured explicitly:

```rust
let worker_threads = std::env::var("TOKIO_WORKER_THREADS")
    .ok()
    .and_then(|t| t.parse().ok())
    .unwrap_or(2);

let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(worker_threads)
    .max_blocking_threads(4)
    .enable_all()
    .build()?;
```

- **`worker_threads = 2`**: Avoids thread contention and context-switching overhead on 2-vCPU systems.
- **`max_blocking_threads = 4`**: Restricts the maximum number of asynchronous blocking tasks spawned for subprocess spawning and disk operations.
- **Process Semaphore**: Subprocess calls to `yt-dlp` are regulated by a global `tokio::sync::Semaphore(1)`. This prevents CPU spikes from crashing the node when multiple users request playlists simultaneously.

---

## 4. The Audio Pipeline & Interception Flow

axAudio hooks directly into the Symphonia / Songbird audio decoding chain. When a track is requested, the system constructs a playable stream and intercepts the raw PCM data:

```
[Remote Stream / CDN URL]
           │
           ▼ (HTTP GET via Reqwest)
[Songbird HttpRequest Input]
           │
           ▼
[Symphonia Probe & Demuxer] (MP3, AAC, FLAC, Opus, OGG)
           │
           ▼
┌───────────────────────────────────────────────────────────────┐
│ DspDecoder (Wrapper around Box<dyn Decoder>)                  │
│                                                               │
│  1. packet = inner.decode(packet)                             │
│  2. If DSP is flat: return decoded frame directly (zero copy) │
│  3. Convert to planar f32 buffer (left, right)                │
│  4. dsp.process_planar(left, right):                          │
│     • Automatic Headroom Attenuation                          │
│     • 15-Band TDF-II Biquad Equalizer                         │
│     • Mid/Side 3D Spatial Widening                            │
│     • Acoustic Crossfeed & Sub-Bass Exciter                   │
│     • Soft-Clip Peak Limiter (tanh)                           │
│  5. Return modified AudioBufferRef::F32                       │
└───────────────────────────────┬───────────────────────────────┘
                                │
                                ▼
               [Songbird Mixer & Resampler (48 kHz)]
                                │
                                ▼
               [Opus Encoder (128 kbps default)]
                                │
                                ▼
               [RTP Packetizer & Voice Encryptor]
                                │
                                ▼ (50 Hz UDP Socket)
                 [Discord Voice Gateway Server]
```

### The `DspDecoder` Interceptor

Songbird provides an extensible decoder slot inside `songbird::input::Parsed`. axAudio uses `std::mem::replace` to wrap the native format decoder with `DspDecoder`:

```rust
pub fn inject_dsp(parsed: &mut songbird::input::Parsed, dsp: Arc<Mutex<DspChain>>) {
    let dummy = Box::new(DummyDecoder);
    let original = std::mem::replace(&mut parsed.decoder, dummy);
    parsed.decoder = Box::new(DspDecoder::new(original, dsp));
}
```

This guarantees:
- **Format Agnostic**: It works uniformly whether the input is raw Opus, an MP3 stream, FLAC, or an AAC YouTube feed.
- **Transparent Fallback**: When the DSP chain is disabled or in `flat` mode, the decoder bypasses all floating-point math and passes raw samples directly to the driver with zero CPU overhead.

---

## 5. Songbird Voice Driver Integration

Discord voice communication operates over UDP using the Real-time Transport Protocol (RTP) encrypted with modern symmetric ciphers.

### Voice Connection Lifecycle

1. **Signaling**: The bot client connects to the Discord Gateway and receives a voice server update (`endpoint`, `token`, `session_id`).
2. **Driver Initialization**: The bot issues a `POST /v1/voice/join` request containing the connection payload. axAudio instantiates or updates a `songbird::driver::Driver` instance.
3. **UDP Handshake**: Songbird initiates the voice gateway WebSocket handshake, performs IP discovery via STUN over UDP, and negotiates the encryption protocol (`aead_xchacha20_poly1305_rtpsize` or `aead_aes256_gcm_rtpsize`).
4. **Frame Dispatcher**: A high-resolution 50 Hz timer awakens every 20 milliseconds to read exactly 960 stereo samples (at 48 kHz), encode them to Opus, encrypt the packet, and dispatch it via UDP.

### Headroom and Soft-Clipping Protection

To prevent harsh digital clipping when users apply high EQ gains or when tracks have excessive master levels, axAudio configures the Songbird driver with:

```rust
let mut config = songbird::Config::default();
config.preallocated_tracks = 1;
config.use_softclip = true;
```

This applies a hyperbolic tangent (`tanh`) compression curve to any residual peaks that exceed 0 dBFS prior to the Opus encoder.

---

## 6. Concurrency & State Management

All shared state in axAudio is hosted in thread-safe, lock-free or fine-grained concurrent data structures:

```rust
pub struct AppState {
    pub players: Arc<DashMap<u64, Arc<Mutex<Driver>>>>,
    pub dsp_chains: Arc<DashMap<u64, Arc<std::sync::Mutex<DspChain>>>>,
    pub pending_playlists: Arc<DashMap<u64, Vec<FlatPlaylistItem>>>,
    pub load_epochs: Arc<DashMap<u64, Arc<AtomicUsize>>>,
    pub event_tx: broadcast::Sender<PlayerEvent>,
    pub resolver: Arc<SourceResolver>,
    pub http_client: reqwest::Client,
    pub start_time: Instant,
}
```

### Epoch-Based Playlist Cancellation

When a user enqueues a large playlist (e.g., 50 songs), a background task resolves tracks asynchronously. If the user calls `/v1/player/control` with `action: "stop"` or sends a new song with `replace: true`, the background task must terminate immediately without resolving further URLs.

axAudio achieves this through **Load Epochs**:
1. Every new playback or stop command increments an atomic counter (`state.next_epoch(guild_id)`).
2. The background worker checks `current_epoch == my_epoch` before each track resolution.
3. If the epoch changed, the worker aborts immediately and cleans up pending items.
