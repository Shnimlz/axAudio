# Plan de Implementación: axAudio Advanced Features

**Fecha**: 2026-10-26  
**Objetivo**: Implementar las 5 grandes áreas de ampliación de axAudio: filtros avanzados de ecualización, endpoints granulares de DSP, persistencia de configuración por guild/bot, motor de streaming en vivo con extracción de metadatos ICY/Shoutcast en tiempo real, y optimización de códecs/presets sugeridos.

---

## 1. Alcance y Arquitectura

### Módulos Afectados:
1. **`src/dsp/biquad.rs` y `src/dsp/equalizer.rs`**:
   - Exponer tipos de filtro (`Peaking`, `LowShelf`, `HighShelf`, `LowPass`, `HighPass`) en `BandSetting`.
   - Soporte para que bandas individuales puedan operar como estantes o filtros de corte.
2. **`src/models.rs`**:
   - `BandSetting` con campo opcional `filter_type: Option<String>`.
   - Payloads para endpoints granulares: `ReverbPayload`, `SpatialPayload`, `TimescalePayload`, `LoudnessPayload`, `ModulationPayload`.
   - `PlayerEvent::MetadataUpdate { guild_id, bot_id, title, artist, raw }`.
   - `TrackResponse` con campo opcional `suggested_preset: Option<String>`.
3. **`src/handlers.rs` y `src/main.rs`**:
   - Nuevas rutas granulares:
     - `POST /v1/player/dsp/reverb`
     - `POST /v1/player/dsp/spatial`
     - `POST /v1/player/dsp/timescale`
     - `POST /v1/player/dsp/loudness`
     - `POST /v1/player/dsp/modulation`
     - `POST /v1/player/dsp/reset`
4. **`src/state.rs`**:
   - Persistencia de configuraciones de guild (`guild_settings.json`): volúmenes, modos de loop, 24/7 y parámetros DSP guardados y cargados al inicio.
5. **`src/sources.rs`**:
   - Demuxer de stream ICY (`IcyStreamReader`) que procesa la cabecera `icy-metaint`, limpia los metadatos intercalados y emite eventos WebSocket con el nombre de la canción en tiempo real mientras entrega audio PCM/Opus limpio a Symphonia.
   - Selección optimizada de formatos en `yt-dlp` (`ba[acodec=opus]/ba`).
   - Detección de `suggested_preset` basada en tags/títulos.
6. **Tests y Documentación**:
   - Tests unitarios y de integración para cada componente nuevo.
   - Actualización de `README.md` y `docs/api.md`.
   - Validación en `vps-local`.

---

## 2. Fases de Ejecución

### Fase 1: Filtros de Ecualización Avanzados (LowShelf, HighShelf, LowPass, HighPass)
- [ ] En `src/dsp/equalizer.rs`:
  - Agregar `filter_type: Option<String>` en `BandSetting`.
  - Mapear strings a `FilterType`: `"peaking"`, `"low_shelf"`, `"high_shelf"`, `"low_pass"`, `"high_pass"`.
  - En `Equalizer::set_band`: permitir cambiar el tipo de filtro de la banda.
- [ ] En `src/dsp/mod.rs`:
  - Pruebas de integración: validar corte drástico con `LowPass` a 1 kHz frente a armónicos de 4 kHz, y realce con `LowShelf`.

### Fase 2: Endpoints Granulares de DSP (`/v1/player/dsp/*`)
- [ ] En `src/models.rs`:
  - `ReverbPayload`, `SpatialPayload`, `TimescalePayload`, `LoudnessPayload`, `ModulationPayload`.
- [ ] En `src/handlers.rs`:
  - `player_dsp_reverb`, `player_dsp_spatial`, `player_dsp_timescale`, `player_dsp_loudness`, `player_dsp_modulation`, `player_dsp_reset`.
- [ ] En `src/main.rs`:
  - Registrar las rutas bajo `protected_routes`.

### Fase 3: Persistencia de Configuración de Servidores (`guild_settings.json`)
- [ ] En `src/state.rs`:
  - Estructura `PersistedGuildConfig { volume, loop_mode, twenty_four_seven, dsp_params }`.
  - Carga al inicio desde `settings/guild_settings.json` (configurable vía `AXAUDIO_GUILD_SETTINGS_FILE`).
  - Sincronización en segundo plano con debounce / atómica ante cambios de configuración.

### Fase 4: Streaming en Vivo con Metadatos ICY/Shoutcast
- [ ] En `src/models.rs`:
  - Añadir `PlayerEvent::MetadataUpdate`.
- [ ] En `src/sources.rs`:
  - Crear lector `IcyDemuxer` o streaming wrapper que capture cabeceras `Icy-MetaData: 1` y parse `StreamTitle`.
  - Al detectar cambio de metadata, emitir `PlayerEvent::MetadataUpdate` al bus WebSocket.

### Fase 5: Formatos Nativos y Presets Sugeridos
- [ ] En `src/sources.rs`:
  - Priorizar formato de audio Opus (`format: ba[acodec=opus]/ba`).
  - Analizar título y tags para asignar `suggested_preset` (ej. "bass", "lofi", "vocal", "acoustic").

### Fase 6: Verificación y Despliegue en `vps-local`
- [ ] Ejecutar `cargo test` localmente.
- [ ] Probar compilación o ejecución en `vps-local` si se requiere.
- [ ] Actualizar documentación técnica.
