# Multi-Bot Session Isolation & Long-Running Loop/24-7 Playback Healing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide full multi-bot session isolation per guild by keying axAudio players on `(guild_id, bot_id)` and eliminate silence/stalls during long-running 24/7 and loop playback by proactively refreshing expired CDN stream URLs, making auto-healing resilient across infinite loops, and respecting 24/7 idle protection.

**Architecture:** 
1. Introduce `PlayerKey { guild_id: u64, bot_id: u64 }` across `AppState`, `models`, and `handlers`, supporting automatic backward-compatible resolution when `bot_id` is omitted.
2. Add URL expiration detection (`is_url_expired_or_expiring`) to `TrackMetadata` using YouTube's `expire=` unix timestamp parameter and resolution age.
3. Reset auto-heal `retried` latch upon successful playback start (`TrackPlayNotifier`), allowing repeated 6-hour CDN stream renewal indefinitely without infinite retry loops on dead URLs.
4. Add first-class `24/7` mode support to `AppState` and GC (`check_idle_cleanup`), preventing idle disconnects when 24/7 or active loop mode is configured.

**Tech Stack:** Rust 2024, Tokio, Axum 0.8, Songbird 0.6, Symphonia 0.5, DashMap 6, Serde.

**Spec:** Direct user requirements:
1. "Puedes verificar porque cuando agrego una cancion y este pasa muchas horas termina en que ya no se escucha la cancion, si es que tenia el modo en 24/7 o en loop."
2. "Tambien añade en los parametros de player el id del bot para que cuando hayan dos bots con el mismo servidor de axaudio no se vaya la sesion a otro bot sin importar si estas en el mismo servidor que el bot que ya tienen una sesion activa."

## Global Constraints
- Preserve backward compatibility for single-bot setups where `bot_id` is omitted from payloads.
- All snowflake IDs (`guild_id`, `channel_id`, `user_id`, `bot_id`) must support both JSON numbers and numeric strings via flexible deserializers.
- Zero audio stutter during track transitions; preserve existing gapless prefetch architecture.

## Review Focus
1. Payloads without `bot_id`: seamlessly resolve to the active guild player if only one exists, or fallback to `(guild_id, 0)`.
2. Two bots joining the exact same `guild_id`: separate `PlayerKey` entries created; neither bot interrupts, replaces, or disconnects the other bot's audio stream.
3. Looping track after 6+ hours: YouTube CDN signed URL expiration handled automatically; playback continues without silence.
4. Auto-healing safety: broken/deleted videos must not enter infinite retry loops (still bounded by 1 retry), while legitimately playing tracks have their retry allowance renewed upon playing.
5. Inactive guild GC: `check_idle_cleanup` must never disconnect drivers that have `is_247 == true` or `loop_mode != LoopMode::Off`.

---

### Task 1: Add `PlayerKey` and Flexible Optional Deserializer in `models.rs`

**Files:**
- Modify: `src/models.rs:1-60`
- Test: `src/models.rs` (unit tests)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
  pub struct PlayerKey {
      pub guild_id: u64,
      pub bot_id: u64,
  }
  impl PlayerKey {
      pub fn new(guild_id: u64, bot_id: u64) -> Self;
  }
  pub fn deserialize_optional_u64_flexible<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>;
  ```

- [ ] **Step 1: Write unit tests for `deserialize_optional_u64_flexible` and `PlayerKey`**

Add tests covering number, string, null, empty string, and omitted values for `bot_id`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test models::tests`
Expected: FAIL (types/functions not yet implemented)

- [ ] **Step 3: Implement `PlayerKey` and `deserialize_optional_u64_flexible` in `src/models.rs`**

Add `PlayerKey` struct with `Display`, `Hash`, `Eq`, and `deserialize_optional_u64_flexible`.
Add `bot_id: Option<u64>` (with serde aliases `user_id` and `bot`) to:
- `JoinPayload` (defaulting to `user_id`)
- `PlayPayload`
- `ControlPayload`
- `AutoplayPayload`
- `LeavePayload`
- `QueuePayload`
- `ResolvePayload`
- `SearchPayload`
- `EqPayload`
- `DspPayload`
- `QueueResponse`
- `DspResponse`
- `PlayerEvent` variants (`TrackStart`, `TrackEnd`, `TrackError`)
- Add `TwentyFourSevenPayload` for `/v1/player/247`

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test models::tests`
Expected: PASS

- [ ] **Step 5: Verify models compilation**

Run: `cargo check`
Expected: PASS

---

### Task 2: Migrate `AppState` and Rate Limiter to `PlayerKey` and Add 24/7 State

**Files:**
- Modify: `src/state.rs`
- Modify: `src/rate_limiter.rs`
- Test: `src/state.rs` (unit tests)

**Interfaces:**
- Consumes: `PlayerKey` from `src/models.rs`
- Produces:
  ```rust
  impl AppState {
      pub fn resolve_player_key(&self, guild_id: u64, bot_id: Option<u64>) -> PlayerKey;
      pub fn is_247(&self, key: PlayerKey) -> bool;
      pub fn set_247(&self, key: PlayerKey, enabled: bool);
      pub fn get_volume(&self, key: PlayerKey) -> f32;
      pub fn set_volume(&self, key: PlayerKey, vol: f32);
      pub fn get_loop_mode(&self, key: PlayerKey) -> LoopMode;
      pub fn set_loop_mode(&self, key: PlayerKey, mode: LoopMode);
      pub fn is_autoplay(&self, key: PlayerKey) -> bool;
      pub fn set_autoplay(&self, key: PlayerKey, enabled: bool);
  }
  ```

- [ ] **Step 1: Write unit tests in `src/state.rs` for `resolve_player_key` and 24/7 state**

Test that:
- Explicit `bot_id` gives `PlayerKey(guild, bot)`.
- When `bot_id` is None, single active player for guild is resolved.
- Multiple active players for same guild remain distinct.
- `set_247` and `is_247` work properly.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test state::tests`
Expected: FAIL

- [ ] **Step 3: Update `AppState` maps and methods in `src/state.rs`**

Replace `u64` keys with `PlayerKey` in:
- `players: Arc<DashMap<PlayerKey, Arc<tokio::sync::Mutex<Driver>>>>`
- `dsp_params`, `load_epochs`, `pending_playlists`, `guild_volumes`, `loop_modes`, `autoplay_guilds`, `seen_tracks`, `last_tracks`, `last_activity`, `prefetching_guilds`, `guild_play_options`
- Add `twenty_four_seven_players: Arc<DashSet<PlayerKey>>`
- Implement `resolve_player_key(&self, guild_id: u64, bot_id: Option<u64>) -> PlayerKey`

- [ ] **Step 4: Run tests in `state.rs`**

Run: `cargo test state::tests`
Expected: PASS

---

### Task 3: URL Expiration Detection and Cache Revalidation in `sources.rs`

**Files:**
- Modify: `src/sources.rs`
- Test: `src/sources.rs` (unit tests)

**Interfaces:**
- Produces:
  ```rust
  impl TrackMetadata {
      pub fn is_url_expired_or_expiring(&self) -> bool;
  }
  ```

- [ ] **Step 1: Write unit test for `is_url_expired_or_expiring`**

Test:
- A URL with `&expire=<unix_timestamp_past>` returns `true`.
- A URL with `&expire=<now + 300>` (< 10 minutes) returns `true`.
- A URL with `&expire=<now + 21600>` (6 hours in future) returns `false`.
- A URL without `expire=` older than 4 hours returns `true`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test sources::tests::test_url_expiration`
Expected: FAIL

- [ ] **Step 3: Implement `is_url_expired_or_expiring` and cache eviction check**

In `src/sources.rs`:
- Add `resolved_at: Instant` to `TrackMetadata` (`#[serde(skip, default = "Instant::now")]`).
- Implement `is_url_expired_or_expiring(&self) -> bool`.
- In `SourceResolver::resolve_single`, when reading cache (`self.cache.get`), check `!meta.is_url_expired_or_expiring()`. If it is expired/expiring, ignore cache hit and fetch fresh URL.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test sources::tests::test_url_expiration`
Expected: PASS

---

### Task 4: Update Handlers for `PlayerKey`, Multi-Bot Isolation, URL Refresh, and Auto-Heal

**Files:**
- Modify: `src/handlers.rs`
- Modify: `src/main.rs` (register `/v1/player/247`)
- Test: Integration tests for handlers

**Interfaces:**
- Consumes: `PlayerKey`, `AppState` methods, `TrackMetadata::is_url_expired_or_expiring`.
- Produces:
  - `voice_join`: uses `PlayerKey::new(payload.guild_id, payload.bot_id.unwrap_or(payload.user_id))`
  - All player endpoints resolve `PlayerKey` using `state.resolve_player_key(payload.guild_id, payload.bot_id)`
  - `TrackPlayNotifier`: resets `retried` flag to `false` on playback start, allowing future 6-hour renewal
  - `TrackEndNotifier`: in `LoopMode::Queue` or loop re-enqueue, refreshes `stream_url` if expired
  - `check_idle_cleanup`: skips cleanup if `state.is_247(key)` or `loop_mode != LoopMode::Off`
  - `player_247`: handles toggle for 24/7 mode

- [ ] **Step 1: Update event notifiers in `src/handlers.rs`**
  - Change `guild_id: u64` in `TrackPlayNotifier`, `TrackEndNotifier`, `TrackErrorNotifier` to `key: PlayerKey`.
  - Pass `retried: Arc<AtomicBool>` to `TrackPlayNotifier` and reset it to `false` in `act()` on `TrackEvent::Play`.
  - In `TrackEndNotifier` for `LoopMode::Queue`, re-resolve `meta.source_url` if `meta.is_url_expired_or_expiring()`.
  - In `TrackErrorNotifier`, keep `retried` check to prevent fast loops, but with the `TrackPlayNotifier` reset, long loops survive 6-hour CDN renewals forever.

- [ ] **Step 2: Update all endpoints in `src/handlers.rs` to use `PlayerKey`**
  - `voice_join`: keys driver by `PlayerKey::new(payload.guild_id, payload.bot_id.unwrap_or(payload.user_id))`.
  - `voice_leave`: leaves only the targeted `PlayerKey`.
  - `player_play`, `player_queue`, `player_control`, `player_seek`, `player_shuffle`, `player_loop`, `player_autoplay`, `player_eq`, `player_dsp`: resolve `PlayerKey` with `state.resolve_player_key(payload.guild_id, payload.bot_id)`.
  - Add `player_247` handler and add `"247"` / `"24_7"` action to `player_control`.
  - In `cleanup_guild_player`, accept `PlayerKey`.
  - In `run_prefetch_monitor`, iterate over `PlayerKey`s and check `maybe_prefetch_next_track`, `check_sponsorblock_skip`, and `check_idle_cleanup`.
  - In `check_idle_cleanup`, bail out if `state.is_247(key) || state.get_loop_mode(key) != LoopMode::Off`.

- [ ] **Step 3: Register `/v1/player/247` in `src/main.rs`**

- [ ] **Step 4: Run full test suite and verify compilation**

Run: `cargo test`
Expected: PASS (all tests pass)

---

### Task 5: Full Verification and Documentation Update

**Files:**
- Modify: `README.md`
- Test: Integration tests & manual verification

- [ ] **Step 1: Write integration tests verifying multi-bot separation**
  Test two bots joining the same guild ID concurrently, verifying both drivers coexist in `AppState` without interference.

- [ ] **Step 2: Update `README.md`**
  Document `bot_id` support in all payload examples and table definitions, and document 24/7 mode `/v1/player/247`.

- [ ] **Step 3: Run final cargo test & check**
  Run: `cargo test && cargo check`
  Expected: PASS
