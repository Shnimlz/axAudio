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

    /// Verifica si hay algún procesamiento activo en la cadena DSP
    #[inline]
    pub fn is_active(&self) -> bool {
        self.equalizer.is_active || self.spatial.is_active()
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
        if !self.is_active() {
            return;
        }

        // 1. Compensación de Headroom (Pre-amp automático):
        // Si hay bandas de EQ con ganancia positiva, atenuamos la entrada para
        // garantizar que ningún pico sature ni distorsione el audio
        let max_boost = self.equalizer.gains_db.iter().copied().fold(0.0f32, f32::max);
        if max_boost > 0.2 {
            let preamp = 10.0f32.powf(-max_boost / 20.0);
            for s in left.iter_mut() {
                *s *= preamp;
            }
            for s in right.iter_mut() {
                *s *= preamp;
            }
        }

        // 2. Ecualizador de 15 bandas
        self.equalizer.process_planar(left, right);

        // 3. Spatializer (Mid/Side + Binaural Crossfeed + Bass Exciter)
        self.spatial.process_planar(left, right);

        // 4. Limitador transparente de picos (solo actúa suavemente si excede 0.95)
        let len = left.len().min(right.len());
        for i in 0..len {
            left[i] = soft_limit_peak(left[i]);
            right[i] = soft_limit_peak(right[i]);
        }
    }
}

/// Limitador transparente: lineal hasta 0.95 (-0.45 dBFS); comprime suavemente con tanh solo los picos extremos
#[inline(always)]
fn soft_limit_peak(x: f32) -> f32 {
    const THRESHOLD: f32 = 0.95;
    const MARGIN: f32 = 1.0 - THRESHOLD;
    if x > THRESHOLD {
        THRESHOLD + MARGIN * ((x - THRESHOLD) / MARGIN).tanh()
    } else if x < -THRESHOLD {
        -THRESHOLD - MARGIN * ((-x - THRESHOLD) / MARGIN).tanh()
    } else {
        x
    }
}
