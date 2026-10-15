//! Módulo de Procesamiento de Señales Digitales (DSP) de axAudio
//!
//! Integra ecualización de 15 bandas, expansión espacial Mid/Side,
//! modelado binaural tipo Dolby Atmos y limitador analógico soft-clip.

pub mod biquad;
pub mod decoder;
pub mod equalizer;
pub mod presets;
pub mod spatial;

pub use biquad::{FilterType, StereoBiquad};
pub use decoder::{inject_dsp, DspDecoder};
pub use equalizer::{BandSetting, Equalizer, EQ_FREQUENCIES};
pub use presets::{apply_preset, get_preset, DspPreset};
pub use spatial::{SpatialProcessor, SpatialSettings};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DspStatus {
    pub active_preset: Option<String>,
    pub stereo_width: f32,
    pub binaural_crossfeed: bool,
    pub bass_exciter: bool,
    pub eq_bands: [f32; 15],
}

/// Cadena de procesamiento maestro en tiempo real
#[derive(Debug, Clone)]
pub struct DspChain {
    pub equalizer: Equalizer,
    pub spatial: SpatialProcessor,
    pub active_preset: Option<String>,
    pub sample_rate: f32,
}

impl DspChain {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            equalizer: Equalizer::new(sample_rate),
            spatial: SpatialProcessor::new(sample_rate),
            active_preset: None,
            sample_rate,
        }
    }

    /// Aplica un preset audiophile (ej: "dolby-atmos", "cinema-3d", "bass-boost-3d")
    pub fn apply_preset(&mut self, preset_name: &str) -> Result<String, String> {
        let desc = presets::apply_preset(preset_name, &mut self.equalizer, &mut self.spatial)?;
        self.active_preset = Some(preset_name.to_lowercase());
        Ok(desc)
    }

    /// Actualiza bandas individuales del ecualizador
    pub fn set_bands(&mut self, bands: &[BandSetting]) {
        self.equalizer.set_bands(bands);
        self.active_preset = None;
    }

    /// Configura los parámetros espaciales
    pub fn set_spatial(&mut self, settings: SpatialSettings) {
        self.spatial.set_settings(settings);
        self.active_preset = None;
    }

    /// Restaura la señal a neutro / plano
    pub fn reset_flat(&mut self) {
        self.equalizer.reset_flat();
        self.spatial.reset();
        self.active_preset = Some("flat".to_string());
    }

    /// Obtiene el estado actual para telemetría y API
    pub fn get_status(&self) -> DspStatus {
        DspStatus {
            active_preset: self.active_preset.clone(),
            stereo_width: self.spatial.settings.stereo_width,
            binaural_crossfeed: self.spatial.settings.binaural_crossfeed,
            bass_exciter: self.spatial.settings.bass_exciter,
            eq_bands: self.equalizer.gains_db,
        }
    }

    /// Procesa un bloque de audio estéreo planar in-place a través de toda la cadena DSP
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        // 1. Ecualizador de 15 bandas
        self.equalizer.process_planar(left, right);

        // 2. Spatializer (Mid/Side + Binaural Crossfeed + Bass Exciter)
        self.spatial.process_planar(left, right);

        // 3. Tanh Soft-Clipping de protección para evitar distorsión inter-sample digital
        let len = left.len().min(right.len());
        for i in 0..len {
            left[i] = (left[i] * 0.95).tanh();
            right[i] = (right[i] * 0.95).tanh();
        }
    }
}
