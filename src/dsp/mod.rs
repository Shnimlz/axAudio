//! Módulo de Procesamiento de Señales Digitales (DSP) de axAudio
//!
//! Integra ecualización de 15 bandas, expansión espacial Mid/Side,
//! modelado acústico 3D y limitador analógico soft-clip.

#![allow(unused_imports, dead_code)]

pub mod biquad;
pub mod decoder;
pub mod equalizer;
pub mod loudness;
pub mod modulation;
pub mod presets;
pub mod spatial;
pub mod timescale;

pub use biquad::{FilterType, StereoBiquad};
pub use decoder::{inject_dsp, DspDecoder};
pub use equalizer::{BandSetting, Equalizer, EQ_FREQUENCIES};
pub use loudness::{LoudnessNormalizer, LoudnessSettings};
pub use modulation::{
    ModulationProcessor, ModulationSettings, RotationProcessor, TremoloProcessor, VibratoProcessor,
};
pub use presets::{apply_preset, apply_preset_to_params, get_preset, DspPreset};
pub use spatial::{SpatialProcessor, SpatialSettings};
pub use timescale::{TimescaleProcessor, TimescaleSettings};

use serde::{Deserialize, Serialize};

/// Parámetros atómicos de configuración DSP compartidos por Guild
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspParams {
    pub gains_db: [f32; 15],
    pub spatial: SpatialSettings,
    pub modulation: ModulationSettings,
    pub loudness: LoudnessSettings,
    pub timescale: TimescaleSettings,
    pub active_preset: Option<String>,
    pub version: u64,
}

impl Default for DspParams {
    fn default() -> Self {
        Self {
            gains_db: [0.0; 15],
            spatial: SpatialSettings::default(),
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
            active_preset: None,
            version: 0,
        }
    }
}

impl DspParams {
    pub fn set_bands(&mut self, bands: &[BandSetting]) {
        for b in bands {
            if b.band < 15 {
                if let Some(db) = b.gain_db {
                    self.gains_db[b.band] = db.clamp(-15.0, 15.0);
                } else if let Some(linear) = b.gain {
                    let db = if linear >= 0.0 {
                        linear * 12.0
                    } else {
                        linear * 48.0
                    };
                    self.gains_db[b.band] = db.clamp(-15.0, 15.0);
                }
            }
        }
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_spatial(&mut self, spatial: SpatialSettings) {
        self.spatial = spatial;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_modulation(&mut self, modulation: ModulationSettings) {
        self.modulation = modulation;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_loudness(&mut self, loudness: LoudnessSettings) {
        self.loudness = loudness;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_timescale(&mut self, timescale: TimescaleSettings) {
        self.timescale = timescale;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn get_status(&self) -> DspStatus {
        DspStatus {
            active_preset: self.active_preset.clone(),
            stereo_width: self.spatial.stereo_width,
            binaural_crossfeed: self.spatial.binaural_crossfeed,
            bass_exciter: self.spatial.bass_exciter,
            eq_bands: self.gains_db,
            rotation: self.modulation.rotation,
            rotation_speed_hz: self.modulation.rotation_speed_hz,
            tremolo: self.modulation.tremolo,
            vibrato: self.modulation.vibrato,
            loudness: self.loudness.enabled,
            target_lufs: self.loudness.target_lufs,
            speed: self.timescale.speed,
            pitch: self.timescale.pitch,
            rate: self.timescale.rate,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DspStatus {
    pub active_preset: Option<String>,
    pub stereo_width: f32,
    pub binaural_crossfeed: bool,
    pub bass_exciter: bool,
    pub eq_bands: [f32; 15],
    pub rotation: bool,
    pub rotation_speed_hz: f32,
    pub tremolo: bool,
    pub vibrato: bool,
    pub loudness: bool,
    pub target_lufs: f32,
    pub speed: f32,
    pub pitch: f32,
    pub rate: f32,
}

/// Cadena de procesamiento maestro en tiempo real (poseída localmente por cada decodificador)
#[derive(Debug, Clone)]
pub struct DspChain {
    pub equalizer: Equalizer,
    pub spatial: SpatialProcessor,
    pub modulation: ModulationProcessor,
    pub loudness: LoudnessNormalizer,
    pub timescale: TimescaleProcessor,
    pub sample_rate: f32,
    pub preamp: f32,
}

impl DspChain {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            equalizer: Equalizer::new(sample_rate),
            spatial: SpatialProcessor::new(sample_rate),
            modulation: ModulationProcessor::new(sample_rate),
            loudness: LoudnessNormalizer::new(sample_rate),
            timescale: TimescaleProcessor::new(),
            sample_rate,
            preamp: 1.0,
        }
    }

    /// Reconstruye los filtros ante cambios de parámetros o tasa de muestreo del audio entrante
    pub fn rebuild(&mut self, params: &DspParams, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.equalizer.update_sample_rate(sample_rate);
        for (i, &gain) in params.gains_db.iter().enumerate() {
            self.equalizer.set_band_db(i, gain);
        }
        self.spatial.update_sample_rate(sample_rate);
        self.spatial.set_settings(params.spatial.clone());

        self.modulation.update_sample_rate(sample_rate);
        self.modulation.set_settings(params.modulation.clone());

        self.loudness.update_sample_rate(sample_rate);
        self.loudness.set_settings(params.loudness.clone());

        self.timescale.set_settings(params.timescale.clone());

        // Cálculo analítico exacto de pico de magnitud combinada para pre-amp headroom:
        // Evaluamos la curva combinada de los 15 filtros más la ganancia lateral Mid/Side
        let eq_peak_db = self.equalizer.compute_max_peak_gain_db();
        let width_boost_db = if params.spatial.stereo_width > 1.0 {
            (params.spatial.stereo_width - 1.0) * 1.5
        } else {
            0.0
        };
        let total_boost_db = (eq_peak_db + width_boost_db).max(0.0);
        self.preamp = if total_boost_db > 0.05 {
            10.0f32.powf(-total_boost_db / 20.0)
        } else {
            1.0
        };
    }

    pub fn reset(&mut self) {
        self.equalizer.reset();
        self.spatial.reset();
        self.modulation.reset();
        self.loudness.reset();
        self.timescale.reset();
    }

    /// Verifica si hay algún procesamiento activo en la cadena DSP
    #[inline]
    pub fn is_active(&self) -> bool {
        self.equalizer.is_active
            || self.spatial.is_active()
            || self.modulation.is_active()
            || self.loudness.is_active()
            || self.timescale.is_active()
    }

    /// Procesa un bloque de audio estéreo planar in-place a través de toda la cadena DSP
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.is_active() {
            return;
        }

        // 1. Compensación analítica exacta de Headroom (Pre-amp transparente)
        if (self.preamp - 1.0).abs() > 0.005 {
            for s in left.iter_mut() {
                *s *= self.preamp;
            }
            for s in right.iter_mut() {
                *s *= self.preamp;
            }
        }

        // 2. Ecualizador de 15 bandas (vectorizado SIMD en f64)
        self.equalizer.process_planar(left, right);

        // 3. Spatializer (Mid/Side + Crossfeed unitario + Excitador armónico)
        self.spatial.process_planar(left, right);

        // 4. Modulación y espacialización dinámica (Vibrato -> Tremolo -> 8D Orbital Rotation)
        self.modulation.process_planar(left, right);

        // 5. Normalizador perceptual de volumen EBU R128 (K-weighting + Smooth AGC + True Peak Limiter)
        self.loudness.process_planar(left, right);

        // 6. Limitador suave de pico analógico universal (Soft-Knee Peak Limiter a 0 dBFS)
        // Si EBU R128 está apagado, protege la salida contra saturación digital / inter-sample clipping en Discord Opus
        if !self.loudness.is_active() {
            let len = left.len().min(right.len());
            for i in 0..len {
                if left[i].abs() > 0.95 {
                    let sign = left[i].signum();
                    let excess = left[i].abs() - 0.95;
                    left[i] = sign * (0.95 + 0.048 * (excess / 0.048).tanh());
                }
                if right[i].abs() > 0.95 {
                    let sign = right[i].signum();
                    let excess = right[i].abs() - 0.95;
                    right[i] = sign * (0.95 + 0.048 * (excess / 0.048).tanh());
                }
            }
        }
    }
}
