//! Procesamiento espacial 3D, ampliación Mid/Side y excitador armónico
//!
//! Recrea la acústica espacial 3D y sonido envolvente cinematográfico en audífonos estéreo.
//! Diseñado sin líneas de retardo que produzcan filtrado en peine (comb-filter),
//! eliminando el sonido metálico o a "radio" y logrando máxima coherencia de fase.

use super::biquad::{FilterType, StereoBiquad};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialSettings {
    /// Ancho del campo estéreo: 0.0 (Mono), 1.0 (Normal), 1.35 (Inmersión 3D), 2.0 (Ultra Wide)
    pub stereo_width: f32,
    /// Activa el crossfeed acústico natural (sombra craneal sin desfase comb-filter)
    pub binaural_crossfeed: bool,
    /// Nivel de crossfeed (0.0 a 0.40, default: 0.18)
    pub crossfeed_level: f32,
    /// Activa el excitador armónico de sub-graves (bajos profundos y viscerales)
    pub bass_exciter: bool,
    /// Cantidad de armónicos generados (0.0 a 1.0, default: 0.35)
    pub bass_exciter_drive: f32,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        Self {
            stereo_width: 1.0,
            binaural_crossfeed: false,
            crossfeed_level: 0.18,
            bass_exciter: false,
            bass_exciter_drive: 0.35,
        }
    }
}

/// Filtro paso-bajo de 1 polo para la sombra acústica del cráneo (~700 Hz)
///
/// Modela la difracción natural de la cabeza sin añadir retardos artificiales
/// de milisegundos que creen cancelaciones de fase o filtrado en peine.
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

/// Procesador espacial y psicoacústico de alta fidelidad
#[derive(Debug, Clone)]
pub struct SpatialProcessor {
    pub settings: SpatialSettings,
    head_filter_l: OnePoleLowPass,
    head_filter_r: OnePoleLowPass,
    bass_filter: StereoBiquad,
    #[allow(dead_code)]
    pub sample_rate: f32,
}

impl SpatialProcessor {
    pub fn new(sample_rate: f32) -> Self {
        let head_filter_l = OnePoleLowPass::new(700.0, sample_rate);
        let head_filter_r = OnePoleLowPass::new(700.0, sample_rate);

        // Filtro paso-bajo a 85 Hz para aislar el sub-grave que alimentará el excitador armónico
        let bass_filter = StereoBiquad::new(FilterType::LowPass, 85.0, 0.0, sample_rate);

        Self {
            settings: SpatialSettings::default(),
            head_filter_l,
            head_filter_r,
            bass_filter,
            sample_rate,
        }
    }

    pub fn set_settings(&mut self, settings: SpatialSettings) {
        self.settings = settings;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        (self.settings.stereo_width - 1.0).abs() > 0.02
            || self.settings.binaural_crossfeed
            || self.settings.bass_exciter
    }

    pub fn reset(&mut self) {
        self.head_filter_l.reset();
        self.head_filter_r.reset();
        self.bass_filter.reset();
    }

    /// Procesa el audio estéreo aplicando Mid/Side, Crossfeed Acústico y Excitador de Bajos
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        let width = self.settings.stereo_width;
        let do_spatial = (width - 1.0).abs() > 0.02;
        let do_binaural = self.settings.binaural_crossfeed;
        let do_bass = self.settings.bass_exciter;

        if !do_spatial && !do_binaural && !do_bass {
            return;
        }

        let crossfeed = self.settings.crossfeed_level.clamp(0.0, 0.40);
        let bass_drive = self.settings.bass_exciter_drive.clamp(0.0, 1.0);
        let len = left.len().min(right.len());

        // Precalcular factores Mid/Side con compensación de ganancia central
        let (gain_mid, gain_side) = if do_spatial {
            let s_gain = width.max(0.0);
            let m_gain = if width > 1.0 {
                (1.0 - (width - 1.0) * 0.18).max(0.5)
            } else {
                1.0
            };
            (m_gain, s_gain)
        } else {
            (1.0, 1.0)
        };

        for i in 0..len {
            let mut l = left[i];
            let mut r = right[i];

            // 1. Excitador Armónico de Sub-graves (Psychoacoustic Bass)
            // Realza la presencia de sub-graves en auriculares sin distorsión áspera
            if do_bass {
                let (sub_l, sub_r) = self.bass_filter.process_pair(l, r);
                let harm_l = (sub_l * 1.5).tanh();
                let harm_r = (sub_r * 1.5).tanh();
                l += harm_l * bass_drive * 0.15;
                r += harm_r * bass_drive * 0.15;
            }

            // 2. Mid/Side Spatial Widening (expansión estéreo de fase coherente sin delay)
            if do_spatial {
                let mid = 0.5 * (l + r);
                let side = 0.5 * (l - r);
                l = mid * gain_mid + side * gain_side;
                r = mid * gain_mid - side * gain_side;
            }

            // 3. Crossfeed Acústico Natural (Sombra craneal analógica, sin retardos)
            if do_binaural {
                let shadow_l = self.head_filter_l.process(l);
                let shadow_r = self.head_filter_r.process(r);

                l = l * (1.0 - crossfeed * 0.5) + shadow_r * crossfeed;
                r = r * (1.0 - crossfeed * 0.5) + shadow_l * crossfeed;
            }

            left[i] = l;
            right[i] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatial_inactive_bypass() {
        let mut proc = SpatialProcessor::new(48000.0);
        assert!(!proc.is_active());
        let mut l = [0.5, -0.5];
        let mut r = [0.25, -0.25];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5, -0.5]);
        assert_eq!(r, [0.25, -0.25]);
    }

    #[test]
    fn test_spatial_mono_collapse() {
        let mut proc = SpatialProcessor::new(48000.0);
        proc.set_settings(SpatialSettings {
            stereo_width: 0.0,
            ..Default::default()
        });
        assert!(proc.is_active());
        let mut l = [1.0];
        let mut r = [0.0];
        proc.process_planar(&mut l, &mut r);
        // Mono output should be identical for both channels: (1.0 + 0.0) / 2 = 0.5
        assert!((l[0] - 0.5).abs() < 1e-6);
        assert!((r[0] - 0.5).abs() < 1e-6);
    }
}

