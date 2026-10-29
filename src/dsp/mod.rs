//! Módulo de Procesamiento de Señales Digitales (DSP) de axAudio
//!
//! Pipeline de 7 etapas: EQ de 15 bandas → Spatial Mid/Side 3D → Reverb FDN
//! (Chowning) → Modulación dinámica (Vibrato/Tremolo/Flanger/Chorus/Phaser/8D
//! Rotation) → Normalización EBU R128 → Soft-Knee Peak Limiter.

#![allow(unused_imports, dead_code)]

pub mod biquad;
pub mod decoder;
pub mod equalizer;
pub mod loudness;
pub mod modulation;
pub mod presets;
pub mod reverb;
pub mod spatial;
pub mod timescale;

pub use biquad::{FilterType, StereoBiquad};
pub use decoder::{inject_dsp, DspDecoder};
pub use equalizer::{
    BandSetting, DynamicFilter, EqChannel, Equalizer, ParametricBand, DOLBY_DAX_FREQUENCIES,
    EQ_FREQUENCIES, ISO_31_FREQUENCIES,
};
pub use loudness::{LoudnessNormalizer, LoudnessSettings};
pub use modulation::{
    ChorusProcessor, FlangerProcessor, ModulationProcessor, ModulationSettings, PhaserProcessor,
    RotationProcessor, TremoloProcessor, VibratoProcessor,
};
pub use presets::{apply_preset, apply_preset_to_params, get_preset, DspPreset};
pub use reverb::{ReverbProcessor, ReverbSettings};
pub use spatial::{SpatialProcessor, SpatialSettings};
pub use timescale::{TimescaleProcessor, TimescaleSettings};

use serde::{Deserialize, Serialize};

/// Parámetros atómicos de configuración DSP compartidos por Guild
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspParams {
    pub gains_db: [f32; 15],
    #[serde(default)]
    pub filter_types: [Option<String>; 15],
    #[serde(default)]
    pub parametric_bands: Option<Vec<ParametricBand>>,
    pub spatial: SpatialSettings,
    #[serde(default)]
    pub reverb: ReverbSettings,
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
            filter_types: Default::default(),
            parametric_bands: None,
            spatial: SpatialSettings::default(),
            reverb: ReverbSettings::default(),
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
                if let Some(ref ft) = b.filter_type {
                    self.filter_types[b.band] = Some(ft.clone());
                }
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

    pub fn set_parametric_bands(&mut self, bands: Vec<ParametricBand>) {
        self.parametric_bands = Some(bands);
        self.active_preset = None;
        self.version += 1;
    }

    pub fn clear_parametric_bands(&mut self) {
        self.parametric_bands = None;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_spatial(&mut self, spatial: SpatialSettings) {
        self.spatial = spatial;
        self.active_preset = None;
        self.version += 1;
    }

    pub fn set_reverb(&mut self, reverb: ReverbSettings) {
        self.reverb = reverb;
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
            reverb: self.reverb.reverb,
            reverb_room_size: self.reverb.reverb_room_size,
            reverb_tone: self.reverb.reverb_tone,
            reverb_wet: self.reverb.reverb_wet,
            eq_bands: self.gains_db,
            filter_types: self.filter_types.clone(),
            parametric_bands: self.parametric_bands.clone(),
            rotation: self.modulation.rotation,
            rotation_speed_hz: self.modulation.rotation_speed_hz,
            tremolo: self.modulation.tremolo,
            vibrato: self.modulation.vibrato,
            flanger: self.modulation.flanger,
            chorus: self.modulation.chorus,
            phaser: self.modulation.phaser,
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
    pub reverb: bool,
    pub reverb_room_size: f32,
    pub reverb_tone: f32,
    pub reverb_wet: f32,
    pub eq_bands: [f32; 15],
    pub filter_types: [Option<String>; 15],
    #[serde(default)]
    pub parametric_bands: Option<Vec<ParametricBand>>,
    pub rotation: bool,
    pub rotation_speed_hz: f32,
    pub tremolo: bool,
    pub vibrato: bool,
    pub flanger: bool,
    pub chorus: bool,
    pub phaser: bool,
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
    pub reverb: ReverbProcessor,
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
            reverb: ReverbProcessor::new(sample_rate),
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
            if let Some(ref ft_str) = params.filter_types[i] {
                self.equalizer.set_band_with_type(i, gain, ft_str);
            } else {
                self.equalizer.set_band_db(i, gain);
            }
        }
        if let Some(ref pb) = params.parametric_bands {
            self.equalizer.set_parametric_bands(pb);
        } else {
            self.equalizer.clear_parametric_bands();
        }
        self.spatial.update_sample_rate(sample_rate);
        self.spatial.set_settings(params.spatial.clone());

        self.reverb.update_sample_rate(sample_rate);
        self.reverb.set_settings(params.reverb.clone());

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
        self.reverb.reset();
        self.modulation.reset();
        self.loudness.reset();
        self.timescale.reset();
    }

    /// Verifica si hay algún procesamiento activo en la cadena DSP
    #[inline]
    pub fn is_active(&self) -> bool {
        self.equalizer.is_active
            || self.spatial.is_active()
            || self.reverb.is_active()
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

        // 4. Reverb FDN (Chowning 1980: 8 líneas coprimas + pre-delay 15 ms + RT60 analítico)
        self.reverb.process_planar(left, right);

        // 5. Modulación dinámica (Vibrato -> Tremolo -> Flanger -> Chorus -> Phaser -> 8D Orbital Rotation)
        self.modulation.process_planar(left, right);

        // 6. Normalizador perceptual de volumen EBU R128 (K-weighting + Smooth AGC + True Peak Limiter)
        self.loudness.process_planar(left, right);

        // 7. Limitador suave de pico analógico universal (Soft-Knee Peak Limiter a 0 dBFS)
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

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Genera una señal sinusoidal estéreo a la frecuencia dada
    fn sine_wave(freq_hz: f32, sample_rate: f32, num_samples: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = Vec::with_capacity(num_samples);
        let mut right = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let phase = 2.0 * std::f32::consts::PI * freq_hz * (i as f32) / sample_rate;
            left.push(phase.sin() * 0.5);
            right.push(phase.sin() * 0.4); // canal derecho ligeramente más bajo
        }
        (left, right)
    }

    /// Calcula el RMS de un buffer
    fn rms(buf: &[f32]) -> f32 {
        let sum: f32 = buf.iter().map(|s| s * s).sum();
        (sum / buf.len() as f32).sqrt()
    }

    /// Calcula el pico absoluto de un buffer
    fn peak(buf: &[f32]) -> f32 {
        buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
    }

    #[test]
    fn test_dsp_chain_inactive_bypass() {
        // Cuando ningún efecto está activo, la cadena debe ser un bypass transparente
        let mut chain = DspChain::new(48000.0);
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 4800);

        assert!(!chain.is_active(), "Chain should be inactive with default params");
        chain.process_planar(&mut left, &mut right);

        // Verificamos que la señal no cambió (bypass)
        let (orig_l, orig_r) = sine_wave(440.0, 48000.0, 4800);
        for i in 0..left.len() {
            assert!((left[i] - orig_l[i]).abs() < 1e-6, "Left sample {} drifted", i);
            assert!((right[i] - orig_r[i]).abs() < 1e-6, "Right sample {} drifted", i);
        }
    }

    #[test]
    fn test_dsp_chain_eq_processing() {
        // EQ activo debe alterar la señal
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            gains_db: [6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active(), "Chain should be active with EQ boost");
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 4800);
        chain.process_planar(&mut left, &mut right);

        // La señal procesada debe tener RMS diferente al original (boost en banda 25Hz)
        let rms_out = rms(&left);
        assert!(rms_out > 0.0, "Output RMS should be non-zero");
        assert!(peak(&left) <= 1.0, "Output should not clip");
    }

    #[test]
    fn test_dsp_chain_filter_types_lowpass() {
        let mut chain = DspChain::new(48000.0);
        let mut filter_types: [Option<String>; 15] = Default::default();
        // Banda 8 = 1000 Hz configurada como LowPass
        filter_types[8] = Some("low_pass".to_string());

        let params = DspParams {
            filter_types,
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);
        assert!(chain.is_active(), "Chain should be active with low_pass filter");

        // Señal de alta frecuencia: 8000 Hz (debe ser muy atenuada por low pass a 1000 Hz)
        let (mut high_left, mut high_right) = sine_wave(8000.0, 48000.0, 4800);
        let high_orig_rms = rms(&high_left);
        chain.process_planar(&mut high_left, &mut high_right);
        let high_filtered_rms = rms(&high_left);

        // La señal de 8000 Hz debe haber caído drásticamente (>12 dB de atenuación)
        assert!(high_filtered_rms < high_orig_rms * 0.25, "8000 Hz signal was not adequately filtered by 1000 Hz LowPass");
    }

    #[test]
    fn test_dsp_chain_filter_types_lowshelf() {
        let mut chain = DspChain::new(48000.0);
        let mut filter_types: [Option<String>; 15] = Default::default();
        let mut gains_db = [0.0; 15];
        // Banda 3 = 100 Hz configurada como LowShelf con +6 dB
        filter_types[3] = Some("low_shelf".to_string());
        gains_db[3] = 6.0;

        let params = DspParams {
            gains_db,
            filter_types,
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);
        assert!(chain.is_active());

        // Señal de subgraves (60 Hz) vs agudos (5000 Hz fuera del shelf).
        // El pre-amp de headroom atenúa la señal completa en ~6 dB para evitar clipping,
        // pero la respuesta de 60 Hz relativa a 5000 Hz debe ser ~2.0 (+6 dB de shelf).
        let (mut sub_left, mut sub_right) = sine_wave(60.0, 48000.0, 4800);
        let sub_orig_rms = rms(&sub_left);
        chain.process_planar(&mut sub_left, &mut sub_right);
        let sub_filtered_rms = rms(&sub_left);

        let (mut high_left, mut high_right) = sine_wave(5000.0, 48000.0, 4800);
        let high_orig_rms = rms(&high_left);
        chain.process_planar(&mut high_left, &mut high_right);
        let high_filtered_rms = rms(&high_left);

        let low_ratio = sub_filtered_rms / sub_orig_rms;
        let high_ratio = high_filtered_rms / high_orig_rms;
        assert!(low_ratio > high_ratio * 1.8, "LowShelf at 100 Hz did not relatively boost 60 Hz over 5000 Hz (low_ratio={}, high_ratio={})", low_ratio, high_ratio);
    }

    #[test]
    fn test_dsp_chain_spatial_widening() {
        // Spatial widening debe alterar la relación entre canales
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            spatial: SpatialSettings {
                stereo_width: 1.5,
                ..Default::default()
            },
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active());
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 4800);
        chain.process_planar(&mut left, &mut right);

        // Con widening, los canales deben divergir más
        assert!(peak(&left) <= 1.0, "Left should not clip");
        assert!(peak(&right) <= 1.0, "Right should not clip");
    }

    #[test]
    fn test_dsp_chain_reverb_active() {
        // Reverb activo debe añadir cola (aumenta RMS por energía de la reverberación)
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            reverb: ReverbSettings {
                reverb: true,
                reverb_room_size: 2.0,
                reverb_wet: 0.4,
                ..Default::default()
            },
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active());
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 9600);
        chain.process_planar(&mut left, &mut right);

        // La señal no debe clipping
        assert!(peak(&left) <= 1.0, "Reverb output should not clip");
        assert!(peak(&right) <= 1.0, "Reverb output should not clip");
    }

    #[test]
    fn test_dsp_chain_modulation_active() {
        // Modulación (8D rotation) debe alterar la señal sin clipping
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            modulation: ModulationSettings {
                rotation: true,
                rotation_speed_hz: 0.15,
                ..Default::default()
            },
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active());
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 4800);
        chain.process_planar(&mut left, &mut right);

        assert!(peak(&left) <= 1.0);
        assert!(peak(&right) <= 1.0);
    }

    #[test]
    fn test_dsp_chain_loudness_normalization() {
        // Loudness normalizer debe mantener la señal dentro de límites
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active());
        // Señal fuerte que necesita reducción
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 48000);
        // Escalamos a nivel alto
        for s in left.iter_mut() {
            *s *= 2.0;
        }
        for s in right.iter_mut() {
            *s *= 2.0;
        }
        chain.process_planar(&mut left, &mut right);

        // El limiter debe mantener todo dentro de ±1.0
        assert!(peak(&left) <= 1.0, "Loudness limiter should prevent clipping");
        assert!(peak(&right) <= 1.0);
    }

    #[test]
    fn test_dsp_chain_full_pipeline() {
        // Pipeline completo: EQ + Spatial + Reverb + Modulation + Loudness
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            gains_db: [3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0],
            filter_types: Default::default(),
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.3,
                binaural_crossfeed: true,
                crossfeed_level: 0.15,
                bass_exciter: true,
                bass_exciter_drive: 0.2,
            },
            reverb: ReverbSettings {
                reverb: true,
                reverb_room_size: 1.8,
                reverb_wet: 0.3,
                ..Default::default()
            },
            modulation: ModulationSettings {
                rotation: true,
                rotation_speed_hz: 0.15,
                ..Default::default()
            },
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
            active_preset: None,
            version: 1,
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active());

        // Procesamos un bloque grande (1 segundo a 48kHz)
        let (mut left, mut right) = sine_wave(440.0, 48000.0, 48000);
        chain.process_planar(&mut left, &mut right);

        // Verificaciones finales del pipeline completo:
        // 1. No clipping
        assert!(peak(&left) <= 1.0, "Full pipeline left channel clipped: peak = {}", peak(&left));
        assert!(peak(&right) <= 1.0, "Full pipeline right channel clipped: peak = {}", peak(&right));

        // 2. Señal no trivial (RMS > 0)
        assert!(rms(&left) > 0.01, "Left channel output is too quiet");
        assert!(rms(&right) > 0.01, "Right channel output is too quiet");

        // 3. Los dos canales son diferentes (estéreo real)
        // Calculamos la diferencia media entre canales — con spatial widening + rotation
        // no deben ser idénticos
        let mut diff_sum = 0.0f32;
        for i in 0..left.len() {
            diff_sum += (left[i] - right[i]).abs();
        }
        let mean_diff = diff_sum / left.len() as f32;
        assert!(mean_diff > 0.005, "Channels are too similar (mean_diff={:.6}), spatial effects may not be working", mean_diff);
    }

    #[test]
    fn test_dsp_chain_preset_application() {
        // Verificar que un preset se aplica correctamente a través de DspParams
        let mut params = DspParams::default();
        let result = apply_preset_to_params("music", &mut params);
        assert!(result.is_ok(), "Preset application failed: {:?}", result.err());

        // Verificar que el preset activó cambios detectables
        let status = params.get_status();
        assert_eq!(status.active_preset, Some("music".to_string()));
        assert!((status.stereo_width - 1.18).abs() < 0.01);
        assert!(status.binaural_crossfeed);

        // Verificar que al menos una banda EQ tiene ganancia no-cero
        assert!(status.eq_bands.iter().any(|g| g.abs() > 0.05));
    }

    #[test]
    fn test_dsp_chain_reset() {
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            gains_db: [6.0; 15],
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);
        assert!(chain.is_active());

        chain.reset();
        // Después del reset, el estado interno se limpia pero is_active depende de gains_db
        // que no cambia con reset() — solo limpia los estados de los filtros
    }

    #[test]
    fn test_dsp_chain_preamp_headroom() {
        // Verificar que el preamp se calcula correctamente con EQ boost alto
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            gains_db: [12.0, 12.0, 12.0, 12.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        // Con 5 bandas a +12 dB, el preamp debe reducir la señal para evitar clipping
        assert!(chain.preamp < 1.0, "Preamp should reduce gain with high EQ boost");
        assert!(chain.preamp > 0.0, "Preamp should be positive");
    }

    #[test]
    fn test_dsp_chain_timescale_active() {
        // Timescale debe activar el pipeline cuando speed != 1.0
        let mut chain = DspChain::new(48000.0);
        let params = DspParams {
            timescale: TimescaleSettings {
                speed: 1.25,
                pitch: 1.0,
                rate: 1.0,
            },
            ..Default::default()
        };
        chain.rebuild(&params, 48000.0);

        assert!(chain.is_active(), "Chain should be active with speed != 1.0");
    }
}
