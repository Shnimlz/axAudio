# Audio Sources & YouTube Anti-Bot Architecture

This document describes how **axAudio** resolves audio media streams, handles playlists with instant playback, bypasses YouTube anti-bot protections, and manages multi-account cookie pools.

---

## 1. Resolution Pipeline Overview

axAudio handles three primary categories of audio sources:

```
                            INPUT URL / QUERY
                                    │
                    ┌───────────────┴───────────────┐
                    │                               │
        Direct Stream (.mp3, ICY)                   │
                    │                               │
                    ▼                               ▼
      ┌───────────────────────────┐   ┌───────────────────────────┐
      │ Direct Stream Bypass      │   │ Source Resolver           │
      │ • Zero yt-dlp overhead    │   │ • DashMap Cache (4h TTL)  │
      │ • Instant playback        │   │ • Process Semaphore (= 1) │
      └─────────────┬─────────────┘   └─────────────┬─────────────┘
                    │                               │
                    │                               ▼
                    │                 ┌───────────────────────────┐
                    │                 │ Cookie Pool Rotation      │
                    │                 │ • Atomic Round-Robin      │
                    │                 │ • Auto-Refresh Write-Back │
                    │                 └─────────────┬─────────────┘
                    │                               │
                    │                               ▼
                    │                 ┌───────────────────────────┐
                    │                 │ Anti-Bot Bypasses         │
                    │                 │ • Deno JS Engine (EJS)    │
                    │                 │ • PoToken Sidecar (:4416) │
                    │                 └─────────────┬─────────────┘
                    │                               │
                    ▼                               ▼
                 [Songbird HttpRequest Input / Symphonia Decoder]
```

---

## 2. Direct Stream Fast-Path

When a URL points to an Icecast, Shoutcast, or direct media container, axAudio bypasses `yt-dlp` entirely:

```rust
pub fn is_direct_stream(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.ends_with(".mp3")
        || lower.ends_with(".ogg")
        || lower.ends_with(".flac")
        || lower.ends_with(".aac")
        || lower.ends_with(".wav")
        || lower.ends_with(".m4a")
        || lower.ends_with(".opus")
        || lower.contains("stream.zeno.fm")
        || lower.contains("icecast")
        || lower.contains("shoutcast")
}
```

This bypass achieves **0 ms extraction overhead** and initiates streaming instantly over HTTP.

---

## 3. YouTube Extraction & Anti-Bot Infrastructure

YouTube aggressively limits automated streaming requests from datacenter IP addresses (AWS, Oracle, Hetzner, DigitalOcean) using proof-of-origin checks, IP rate limits, and JavaScript signature scrambling.

axAudio overcomes these restrictions through three complementary layers:

### A. Deno Embedded JavaScript Engine
YouTube signatures change dynamically using complex, obfuscated JavaScript routines. Python's default regex extractors fail on these challenges. 

axAudio's runtime environment installs the native **Deno** engine (`/usr/local/bin/deno`). `yt-dlp` utilizes Deno to execute signature deobfuscation scripts in an isolated, sandboxed environment, ensuring extraction success.

### B. Proof of Origin Token (PoToken) Sidecar
Google uses "BotGuard" to challenge headless clients. The `bgutil-ytdlp-pot-provider` sidecar generates valid Proof of Origin Tokens (`po_token`) and visitor IDs.

In `docker-compose.yml`:
```yaml
services:
  axaudio:
    environment:
      - BGUTIL_POT_URL=http://bgutil-pot:4416
    depends_on:
      - bgutil-pot

  bgutil-pot:
    image: brainicism/bgutil-ytdlp-pot-provider:latest
    ports:
      - "4416:4416"
```

axAudio queries this sidecar automatically, appending valid tokens to every YouTube extraction request.

---

## 4. Multi-Account Cookie Pool Rotation

Single-account cookie configurations inevitably hit Google rate limits or captcha verification when serving active voice servers. axAudio features a dynamic **Multi-Account Cookie Pool**.

### Scanning & Auto-Discovery
On startup, axAudio aggregates cookie files (`.txt` in Netscape format) from multiple discovery paths:

1. Directory paths: `YTDLP_COOKIES_DIR`, `/app/cookies/*.txt`, `./cookies/*.txt`.
2. Comma-separated list: `YTDLP_COOKIES_POOL="cookie1.txt,cookie2.txt"`.
3. Legacy single file: `/app/cookies.txt`.

### Round-Robin Atomic Rotation
Extractors cycle across discovered cookie files using a thread-safe `AtomicUsize`:

```rust
let pool = self.get_available_cookie_pool().await;
if !pool.is_empty() {
    let idx = self.cookie_rotation_index.fetch_add(1, Ordering::SeqCst);
    let chosen = &pool[idx % pool.len()];
    cmd.arg("--cookies").arg(chosen);
}
```

### In-Place Cookie Persistence & Write-Backs
Google periodically refreshes session tokens during active sessions. `yt-dlp` updates these tokens directly inside the mounted cookie file.

> [!IMPORTANT]
> Always ensure cookie files have write permissions on the host system:
> ```bash
> chmod -R a+rwX ./cookies
> chmod 666 ./cookies.txt
> ```
> If permissions are read-only, `yt-dlp` will fail when attempting to write updated session tokens.

---

## 5. Fast-Start Playlist Architecture

Loading playlists in traditional bots typically freezes the player for 10–30 seconds while all songs are downloaded and indexed.

axAudio employs an asynchronous **two-stage pipeline**:

```
[Playlist URL Received]
         │
         ▼
[yt-dlp --flat-playlist --dump-single-json --playlist-end 25] (1.5s)
         │
         ├─────────────────────────────────────────┐
         ▼                                         ▼
   [First Track URL]                    [Remaining 24 Entries]
         │                                         │
         ▼                                         ▼
[Resolve Direct CDN Stream]             [Insert into Pending Map]
         │                                         │
         ▼                                         ▼
[Enqueue in Songbird & Play NOW]        [Spawn Background Worker]
                                                   │
                                                   ▼
                                       ┌───────────────────────┐
                                       │ Loop with Throttling: │
                                       │ 1. Check Epoch        │
                                       │ 2. Wait if Queue >= 3 │
                                       │ 3. 1.5s CPU Cooldown  │
                                       │ 4. Resolve Next Track │
                                       │ 5. Enqueue & Repeat   │
                                       └───────────────────────┘
```

### Background Throttling Rules:
1. **Queue Buffer Cap**: If the Songbird playback queue already contains 3 or more ready tracks, the background worker pauses until playback advances. This prevents RAM inflation and eliminates redundant queries if users disconnect early.
2. **CPU Breathing Cooldown**: A mandatory 1.5-second cooldown pause is enforced between individual track resolutions, guaranteeing that 1-vCPU VPS instances maintain fluid audio without core saturation.
3. **Epoch Cancellation**: If a user runs `/stop` or replaces playback, the atomic epoch increments, and the background task exits immediately.

---

## 6. In-Memory Stream URL Caching

YouTube video stream URLs (`googlevideo.com`) remain valid for up to 6 hours. axAudio caches resolved streams in a high-speed concurrent `DashMap` with a **4-hour Time-To-Live (TTL)**:

```rust
pub struct SourceResolver {
    cache: Arc<DashMap<String, (TrackMetadata, Instant)>>,
    ttl: Duration, // 4 hours
}
```

Repeated requests for popular songs resolve instantly from RAM with **0 ms network extraction latency**.
