# axAudio Documentation

Welcome to the internal engineering and operational documentation for **axAudio**, a high-performance, bare-metal Discord audio streaming engine written in Rust.

axAudio replaces traditional JVM-based audio nodes (such as Lavalink) with a single, statically compiled binary that boots in under 5 milliseconds, consumes 8–15 MB of resident RAM, and streams real-time 48 kHz Opus audio with zero garbage collection pauses.

---

## System Mental Model

axAudio divides responsibilities cleanly between control signaling and real-time audio transport:

```
                      CONTROL PLANE (HTTP / WS)
  Discord Bot ───────────────────────────────────────────────┐
       │                                                     │
       ├─► POST /v1/voice/join (session, token, endpoint)   │
       ├─► POST /v1/player/play (URL / query)                │
       ├─► POST /v1/player/eq   (Multi-Mode EQ / presets)    ▼
       └─► GET  /v1/events      (WebSocket telemetry)   ┌─────────────┐
                                                        │ Axum Server │
                                                        └──────┬──────┘
                                                               │
                     AUDIO PIPELINE (Async Rust)               │
 ┌─────────────────────────────────────────────────────────────┘
 │
 ▼
┌──────────────────────┐      ┌────────────────────────┐      ┌──────────────────────┐
│   Source Resolver    │ ───► │   Symphonia Decoder    │ ───► │ Multi-Mode DSP Chain │
│ yt-dlp / PoToken /   │      │  (PCM f32 Interceptor) │      │  TDF-II Biquads +    │
│  Multi-Cookie Pool   │      └────────────────────────┘      │  Mid/Side M/S + Limit│
└──────────────────────┘                                      └──────────┬───────────┘
                                                                         │
                     REAL-TIME TRANSPORT (UDP RTP)                       ▼
                                                              ┌──────────────────────┐
Discord Voice Server ◄─────────────────────────────────────── │   Songbird Driver    │
(50 Hz, 20ms Opus frames, ChaCha20-Poly1305 / AES-GCM)        │   (50 Hz Scheduler)  │
                                                              └──────────────────────┘
```

1. **Control Plane (`Axum`)**: Exposes REST endpoints for joining voice channels, enqueueing tracks, manipulating playback state, and configuring real-time DSP filters, accompanied by an event-driven WebSocket broadcast bus (`/v1/events`).
2. **Source Resolver**: An asynchronous extraction layer capable of resolving single tracks, flat playlists, and direct HTTP/ICY streams while bypassing YouTube anti-bot mechanisms using rotating cookie pools and proof-of-origin tokens (`bgutil-pot`).
3. **Decoded Audio Interceptor (`DspDecoder`)**: Seamlessly wraps Symphonia codec decoders, capturing planar 32-bit floating-point audio blocks before they reach the mixer.
4. **Master DSP Chain**: Applies custom Transposed Direct Form II (TDF-II) multi-mode equalization (15-band ISO, Dolby DAX 20 critical bands, ISO 31-band 1/3-octave, dynamic Parametric M/S), phase-coherent Mid/Side stereo widening, acoustic crossfeed, harmonic sub-bass excitation, and soft-clip peak limiting.
5. **Real-time Transport (`Songbird`)**: Maintains the encrypted UDP RTP voice socket directly to Discord's voice servers, pacing packets at exactly 50 Hz (20ms intervals).

---

## Documentation Index

Explore the detailed architecture, mathematical models, API contracts, and operational guides:

| Guide | Description |
| :--- | :--- |
| [**Architecture & Concurrency**](architecture.md) | High-level system design, thread model, zero-GC memory guarantees, Songbird driver integration, and audio packet scheduling. |
| [**Digital Signal Processing (DSP)**](dsp.md) | Mathematical formulation of TDF-II biquad filters, 15-band / Dolby 20-band / ISO-31 / Parametric M/S equalization, Motorola Dolby Atmos calibrations, 3D spatial widening, acoustic crossfeed, and peak limiters. |
| [**REST API Reference**](api.md) | Full endpoint contracts, payload specifications, JSON schemas, status codes, and curl examples. |
| [**WebSocket Telemetry**](telemetry.md) | Real-time event bus (`/v1/events`), packet definitions (`track_start`, `track_end`, `track_error`), and reconnection recovery. |
| [**Sources & YouTube Bypass**](streaming-and-sources.md) | Media resolution architecture, fast-start playlist loading, cookie pool rotation, Deno JS engine, and PoToken sidecar integration. |
| [**Deployment & Operations**](deployment.md) | Docker Compose setup, bare-metal builds, host networking, Tailscale mesh connectivity, and VPS resource optimization. |

---

## Architectural Invariants

When extending or maintaining axAudio, preserve these boundaries:

- **Zero-GC Determinism**: Audio pipeline execution must never allocate dynamically in the per-sample hot loop. All buffers (`f32_buf`) are pre-sized and reused.
- **Planar Isolation**: Channel operations are strictly decoupled into independent contiguous left and right float slices (`&mut [f32]`), ensuring optimal SIMD auto-vectorization on x86_64 (AVX2) and AArch64 (NEON).
- **Sub-Millisecond Non-Blocking API**: The HTTP control plane must never block awaiting audio network packets or synchronous disk operations. Heavy media metadata queries run inside asynchronous workers bounded by process semaphores.
- **Fail-Safe Bypass**: If DSP filters are inactive or flat, the audio stream bypasses float conversions entirely, routing native decoded frames directly to Songbird.
