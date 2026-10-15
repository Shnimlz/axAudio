//! Procesamiento espacial, sonido envolvente 3D binaural y excitador armónico
//!
//! Recrea la psicoacústica de Dolby Atmos / salas de cine en audífonos estéreo.

use super::biquad::{FilterType, StereoBiquad};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialSettings {
    /// Ancho del campo estéreo: 0.0 (Mono), 1.0 (Normal), 1.4 (Cine 3D), 2.0 (Ultra Wide)
    pub stereo_width: f32,
    /// Activa el crossfeed binaural (modelado de retraso interaural y sombra craneal)
    pub binaural_crossfeed: bool,
    /// Nivel de crossfeed (0.0 a 0.5, default: 0.22)
    pub crossfeed_level: f32,
    /// Activa el excitador armónico de sub-graves (bajos viscerales estilo cine)
    pub bass_exciter: bool,
    /// Cantidad de armónicos generados (0.0 a 1.0, default: 0.35)
    pub bass_exciter_drive: f32,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        Self {
            stereo_width: 1.0,
            binaural_crossfeed: false,
            crossfeed_level: 0.22,
            bass_exciter: false,
            bass_exciter_drive: 0.35,
        }
    }
}

/// Buffer circular para retraso de fracciones de milisegundo (ITD - Interaural Time Difference)
#[derive(Debug, Clone)]
struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl DelayLine {
    fn new(delay_samples: usize) -> Self {
        let size = delay_samples.max(1);
        Self {
            buffer: vec![0.0; size],
            write_pos: 0,
        }
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        let out = self.buffer[self.write_pos];
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();
        out
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}

/// Filtro paso-bajo de 1 polo para la sombra acústica del cráneo (~700 Hz)
#[derive(Debug, Clone)]
struct OnePoleLowPass {
    a0: f32,
    b1: f32,
    z1: f32,
}

impl OnePoleLowPass {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let costh = 2.0 - (2.0 * std::f32::consts::PI * cutoff_hz / sample_rate).cos();
        let b1 = (costh * costh - 1.0).sqrt() - costh;
        let a0 = 1.0 + b1;
        Self { a0, b1, z1: 0.0 }
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        self.z1 = input * self.a0 - self.z1 * self.b1;
        self.z1
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
    }
}

/// Procesador espacial y psicoacústico completo
#[derive(Debug, Clone)]
pub struct SpatialProcessor {
    pub settings: SpatialSettings,
    delay_l: DelayLine,
    delay_r: DelayLine,
    head_filter_l: OnePoleLowPass,
    head_filter_r: OnePoleLowPass,
    bass_filter: StereoBiquad,
    sample_rate: f32,
}

impl SpatialProcessor {
    pub fn new(sample_rate: f32) -> Self {
        // ~0.62 ms de retraso = ~30 muestras a 48 kHz (distancia acústica entre oídos promedio)
        let delay_samples = ((sample_rate * 0.00062).round() as usize).max(4);

        let delay_l = DelayLine::new(delay_samples);
        let delay_r = DelayLine::new(delay_samples);
        let head_filter_l = OnePoleLowPass::new(700.0, sample_rate);
        let head_filter_r = OnePoleLowPass::new(700.0, sample_rate);

        // Filtro paso-bajo a 85 Hz para aislar el sub-grave que alimentará el excitador armónico
        let bass_filter = StereoBiquad::new(FilterType::LowPass, 85.0, 0.0, sample_rate);

        Self {
            settings: SpatialSettings::default(),
            delay_l,
            delay_r,
            head_filter_l,
            head_filter_r,
            bass_filter,
            sample_rate,
        }
    }

    pub fn set_settings(&mut self, settings: SpatialSettings) {
        self.settings = settings;
    }

    pub fn reset(&mut self) {
        self.delay_l.reset();
        self.delay_r.reset();
        self.head_filter_l.reset();
        self.head_filter_r.reset();
        self.bass_filter.reset();
    }

    /// Procesa el audio estéreo aplicando Mid/Side, Binaural Crossfeed y Excitador de Bajos
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        let width = self.settings.stereo_width;
        let do_spatial = (width - 1.0).abs() > 0.02;
        let do_binaural = self.settings.binaural_crossfeed;
        let do_bass = self.settings.bass_exciter;

        if !do_spatial && !do_binaural && !do_bass {
            return;
        }

        let crossfeed = self.settings.crossfeed_level.clamp(0.0, 0.45);
        let bass_drive = self.settings.bass_exciter_drive.clamp(0.0, 1.0);
        let len = left.len().min(right.len());

        for i in 0..len {
            let mut l = left[i];
            let mut r = right[i];

            // 1. Excitador Armónico de Sub-graves (Psychoacoustic Bass)
            if do_bass {
                let (sub_l, sub_r) = self.bass_filter.process_pair(l, r);
                // Generar 2do y 3er armónico no-lineal mediante saturación suave tanh
                let harm_l = (sub_l * (1.0 + bass_drive * 3.0)).tanh();
                let harm_r = (sub_r * (1.0 + bass_drive * 3.0)).tanh();
                l += harm_l * bass_drive * 0.4;
                r += harm_r * bass_drive * 0.4;
            }

            // 2. Mid/Side Spatial Widening
            if do_spatial {
                let mid = 0.5 * (l + r);
                let side = 0.5 * (l - r) * width;
                l = mid + side;
                r = mid - side;
            }

            // 3. Binaural Crossfeed (Modelado de Cabeza 3D / Fuera de la Cabeza)
            if do_binaural {
                // El canal izquierdo viaja al oído derecho con retraso y filtro craneal
                let delayed_l = self.delay_l.process(l);
                let shadowed_l = self.head_filter_l.process(delayed_l);

                // El canal derecho viaja al oído izquierdo con retraso y filtro craneal
                let delayed_r = self.delay_r.process(r);
                let shadowed_r = self.head_filter_r.process(delayed_r);

                l = l * (1.0 - crossfeed * 0.5) + shadowed_r * crossfeed;
                r = r * (1.0 - crossfeed * 0.5) + shadowed_l * crossfeed;
            }

            left[i] = l;
            right[i] = r;
        }
    }
}
