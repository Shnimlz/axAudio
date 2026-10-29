//! Ecualizador gráfico y paramétrico de precisión de estudio con procesamiento Mid/Side
//!
//! Soporta tres modos principales de ecualización de alta fidelidad:
//! 1. Modo ISO 15 bandas estándar (25 Hz a 16 kHz) con Q psicoacústico adaptativo (compatible con Lavalink).
//! 2. Modo Dolby DAX 20 bandas críticas psicoacústicas (47 Hz a 19.688 kHz) extraído de Motorola DAX.
//! 3. Modo ISO 31 bandas de 1/3 de octava de grado de masterización (20 Hz a 20 kHz).
//! 4. Modo Paramétrico Completo arbitrario (frecuencia, Q, ganancia, tipo de filtro).
//! 5. Ecualización Mid/Side (M/S): procesa el canal central (Mid/Vocales/Mono) y lateral (Side/Amplitud/Ambiente)
//!    de forma independiente para lograr una separación estéreo holográfica sin artefactos de fase.

use super::biquad::{BiquadCoefficients, BiquadState, FilterType, StereoBiquad};
use serde::{Deserialize, Serialize};

/// Frecuencias estándar ISO / Lavalink de 15 bandas
pub const EQ_FREQUENCIES: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];

/// 20 Bandas críticas psicoacústicas de Dolby Audio Processing (DAP) / DAX v3
/// Extraídas del perfil de audio Motorola (dax-default.xml / libswdap.so)
pub const DOLBY_DAX_FREQUENCIES: [f32; 20] = [
    47.0, 141.0, 234.0, 328.0, 469.0, 656.0, 844.0, 1031.0, 1313.0, 1688.0, 2250.0, 3000.0,
    3750.0, 4688.0, 5813.0, 7125.0, 9000.0, 11250.0, 13875.0, 19688.0,
];

/// 31 Bandas estándar ISO de 1/3 de octava para ecualizadores gráficos de estudio
pub const ISO_31_FREQUENCIES: [f32; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

/// Factor Q calibrado por banda según principios psicoacústicos de masterización
#[inline]
pub fn get_band_q(band: usize) -> f32 {
    match band {
        0 => 0.8,       // 25 Hz: subgrave ultra-bajo, ancho y libre de resonancia inestable
        1 => 1.0,       // 40 Hz: pegada profunda de bombo/sub-bass
        2 => 1.2,       // 63 Hz: bajo eléctrico y sintetizadores
        3 => 1.4,       // 100 Hz: cuerpo de graves
        4 => 1.6,       // 160 Hz: calidez de medios-graves
        5..=9 => 1.8,   // 250 Hz - 2.5 kHz: precisión quirúrgica para rango vocal e instrumental
        10..=12 => 1.5, // 4 kHz - 6.3 kHz: presencia y brillo nítido
        _ => 1.2,       // 10 kHz - 16 kHz: aire sedoso en agudos tipo campana ancha
    }
}

/// Canal destino para filtros paramétricos y Mid/Side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqChannel {
    Stereo, // Ambos canales L/R idénticamente
    Mid,    // Canal central / mono (M = (L+R)/sqrt(2)): diálogos, voces principales, bombos centrados
    Side,   // Canal lateral / estéreo (S = (L-R)/sqrt(2)): reverberación, amplitud, aire 3D
    Left,   // Canal izquierdo exclusivamente
    Right,  // Canal derecho exclusivamente
}

impl Default for EqChannel {
    fn default() -> Self {
        Self::Stereo
    }
}

/// Configuración de una banda estándar de 15 bandas
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandSetting {
    pub band: usize,
    #[serde(default)]
    pub gain: Option<f32>,     // Lineal Lavalink (-0.25 a +1.0)
    #[serde(default)]
    pub gain_db: Option<f32>,  // Decibelios directos (-15.0 a +15.0)
    #[serde(default)]
    pub filter_type: Option<String>, // "peaking" (default), "low_shelf", "high_shelf", "low_pass", "high_pass", "notch", "band_pass"
}

/// Configuración de una banda paramétrica de precisión arbitraria
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParametricBand {
    pub freq: f32,
    pub gain_db: f32,
    #[serde(default = "default_band_q")]
    pub q: f32,
    #[serde(default = "default_filter_type_str")]
    pub filter_type: String,
    #[serde(default)]
    pub channel: EqChannel,
}

fn default_band_q() -> f32 {
    1.0
}

fn default_filter_type_str() -> String {
    "peaking".to_string()
}

/// Filtro dinámico en tiempo real para bandas paramétricas y M/S
#[derive(Debug, Clone)]
pub struct DynamicFilter {
    pub filter_type: FilterType,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    pub channel: EqChannel,
    pub coeffs: BiquadCoefficients,
    pub state_l: BiquadState,
    pub state_r: BiquadState,
    pub active: bool,
}

impl DynamicFilter {
    pub fn new(channel: EqChannel, filter_type: FilterType, freq: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self {
        let coeffs = BiquadCoefficients::compute(filter_type, freq, gain_db, sample_rate);
        let active = matches!(
            filter_type,
            FilterType::LowPass | FilterType::HighPass | FilterType::Notch { .. } | FilterType::BandPass { .. }
        ) || gain_db.abs() > 0.01;
        Self {
            filter_type,
            freq,
            gain_db,
            q,
            channel,
            coeffs,
            state_l: BiquadState::default(),
            state_r: BiquadState::default(),
            active,
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.coeffs = BiquadCoefficients::compute(self.filter_type, self.freq, self.gain_db, sample_rate);
    }

    pub fn reset(&mut self) {
        self.state_l.reset();
        self.state_r.reset();
    }
}

/// Motor de Ecualización Avanzado de axAudio
#[derive(Debug, Clone)]
pub struct Equalizer {
    pub filters: Vec<StereoBiquad>,
    pub filter_types: [FilterType; 15],
    pub gains_db: [f32; 15],
    pub parametric_filters: Vec<DynamicFilter>,
    pub sample_rate: f32,
    pub is_active: bool,
    pub has_mid_side: bool,
}

impl Equalizer {
    /// Convierte una cadena de texto en un tipo de filtro de biquad
    pub fn parse_custom_filter_type(s: &str, q: f32) -> FilterType {
        let clean = s.to_lowercase().replace(['_', '-'], "");
        match clean.as_str() {
            "lowshelf" | "shelfdown" | "bassshelf" => FilterType::LowShelf,
            "highshelf" | "shelfup" | "trebleshelf" => FilterType::HighShelf,
            "lowpass" | "lp" | "hicut" => FilterType::LowPass,
            "highpass" | "hp" | "lowcut" => FilterType::HighPass,
            "notch" | "bandstop" => FilterType::Notch { q: q.max(0.1) },
            "bandpass" | "bp" => FilterType::BandPass { q: q.max(0.1) },
            _ => FilterType::Peaking { q: q.max(0.1) },
        }
    }

    pub fn parse_filter_type(band: usize, s: &str) -> FilterType {
        Self::parse_custom_filter_type(s, get_band_q(band))
    }

    pub fn new(sample_rate: f32) -> Self {
        let mut filters = Vec::with_capacity(15);
        let mut filter_types = [FilterType::Peaking { q: 1.0 }; 15];
        for (band, &freq) in EQ_FREQUENCIES.iter().enumerate() {
            let ft = FilterType::Peaking { q: get_band_q(band) };
            filter_types[band] = ft;
            filters.push(StereoBiquad::new(ft, freq, 0.0, sample_rate));
        }

        Self {
            filters,
            filter_types,
            gains_db: [0.0; 15],
            parametric_filters: Vec::new(),
            sample_rate,
            is_active: false,
            has_mid_side: false,
        }
    }

    /// Adapta los filtros si la tasa de muestreo cambia
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 1.0 {
            self.sample_rate = sample_rate;
            for (i, filter) in self.filters.iter_mut().enumerate() {
                let freq = EQ_FREQUENCIES[i];
                filter.update(self.filter_types[i], freq, self.gains_db[i], sample_rate);
            }
            for pf in self.parametric_filters.iter_mut() {
                pf.update_sample_rate(sample_rate);
            }
        }
    }

    /// Configura una banda específica (0 a 14) con ganancia en dB (tipo Peaking por defecto)
    pub fn set_band_db(&mut self, band: usize, gain_db: f32) {
        if band < 15 {
            let clamped = gain_db.clamp(-15.0, 15.0);
            self.gains_db[band] = clamped;
            let freq = EQ_FREQUENCIES[band];
            self.filters[band].update(self.filter_types[band], freq, clamped, self.sample_rate);
            self.check_active();
        }
    }

    /// Configura una banda con ganancia en dB y tipo de filtro explícito
    pub fn set_band_with_type(&mut self, band: usize, gain_db: f32, filter_type_str: &str) {
        if band < 15 {
            let clamped = gain_db.clamp(-15.0, 15.0);
            self.gains_db[band] = clamped;
            let ft = Self::parse_filter_type(band, filter_type_str);
            self.filter_types[band] = ft;
            let freq = EQ_FREQUENCIES[band];
            self.filters[band].update(ft, freq, clamped, self.sample_rate);
            self.check_active();
        }
    }

    /// Configura una banda usando la escala Lavalink (-0.25 a +1.0) convirtiendo a dB
    pub fn set_band_lavalink(&mut self, band: usize, gain_linear: f32) {
        if band < 15 {
            let db = if gain_linear >= 0.0 {
                gain_linear * 12.0
            } else {
                gain_linear * 48.0
            };
            self.set_band_db(band, db);
        }
    }

    /// Aplica una lista de bandas de forma atómica sin ambigüedades de escala
    pub fn set_bands(&mut self, bands: &[BandSetting]) {
        for b in bands {
            let db = if let Some(d) = b.gain_db {
                d
            } else if let Some(linear) = b.gain {
                if linear >= 0.0 {
                    linear * 12.0
                } else {
                    linear * 48.0
                }
            } else {
                0.0
            };

            if let Some(ref ft_str) = b.filter_type {
                self.set_band_with_type(b.band, db, ft_str);
            } else {
                self.set_band_db(b.band, db);
            }
        }
        self.check_active();
    }

    /// Configura una lista de bandas paramétricas de frecuencia, Q y canal libre
    pub fn set_parametric_bands(&mut self, bands: &[ParametricBand]) {
        self.parametric_filters = bands
            .iter()
            .map(|b| {
                let ft = Self::parse_custom_filter_type(&b.filter_type, b.q);
                DynamicFilter::new(
                    b.channel,
                    ft,
                    b.freq.clamp(20.0, 22000.0),
                    b.gain_db.clamp(-24.0, 24.0),
                    b.q.clamp(0.1, 10.0),
                    self.sample_rate,
                )
            })
            .collect();
        self.check_active();
    }

    /// Elimina todas las bandas paramétricas activas
    pub fn clear_parametric_bands(&mut self) {
        self.parametric_filters.clear();
        self.check_active();
    }

    /// Configura las 20 bandas psicoacústicas de Dolby DAX
    pub fn set_dolby_20_bands(&mut self, gains: &[f32; 20]) {
        let bands: Vec<ParametricBand> = DOLBY_DAX_FREQUENCIES
            .iter()
            .zip(gains.iter())
            .map(|(&freq, &gain_db)| ParametricBand {
                freq,
                gain_db,
                q: 1.4,
                filter_type: "peaking".to_string(),
                channel: EqChannel::Stereo,
            })
            .collect();
        self.set_parametric_bands(&bands);
    }

    /// Configura las 31 bandas ISO de 1/3 de octava
    pub fn set_iso_31_bands(&mut self, gains: &[f32; 31]) {
        let bands: Vec<ParametricBand> = ISO_31_FREQUENCIES
            .iter()
            .zip(gains.iter())
            .map(|(&freq, &gain_db)| ParametricBand {
                freq,
                gain_db,
                q: 2.0,
                filter_type: "peaking".to_string(),
                channel: EqChannel::Stereo,
            })
            .collect();
        self.set_parametric_bands(&bands);
    }

    /// Calcula la ganancia pico real de la curva combinada de todos los filtros activos
    /// (estándar + paramétricos + Mid/Side) para compensación analítica contra saturación
    pub fn compute_max_peak_gain_db(&self) -> f32 {
        let mut active_coeffs: Vec<&BiquadCoefficients> = self
            .filters
            .iter()
            .filter(|f| f.active)
            .map(|f| &f.coeffs)
            .collect();

        for pf in &self.parametric_filters {
            if pf.active {
                active_coeffs.push(&pf.coeffs);
            }
        }

        if active_coeffs.is_empty() {
            return 0.0;
        }

        let mut max_gain_db = 0.0f64;
        let sr = self.sample_rate as f64;
        let min_freq = 20.0f64;
        let max_freq = 20000.0f64.min(sr * 0.49);
        let steps = 64;
        let log_min = min_freq.ln();
        let log_max = max_freq.ln();

        for i in 0..=steps {
            let f = (log_min + (log_max - log_min) * (i as f64 / steps as f64)).exp();
            let w = 2.0 * std::f64::consts::PI * f / sr;
            let sum_db: f64 = active_coeffs.iter().map(|coeffs| coeffs.mag_db(w)).sum();
            if sum_db > max_gain_db {
                max_gain_db = sum_db;
            }
        }

        max_gain_db as f32
    }

    /// Restablece todas las bandas estándar a plano (0 dB) y elimina bandas paramétricas
    pub fn reset_flat(&mut self) {
        self.gains_db = [0.0; 15];
        for (i, filter) in self.filters.iter_mut().enumerate() {
            let ft = FilterType::Peaking { q: get_band_q(i) };
            self.filter_types[i] = ft;
            filter.update(ft, EQ_FREQUENCIES[i], 0.0, self.sample_rate);
            filter.reset();
        }
        self.parametric_filters.clear();
        self.is_active = false;
        self.has_mid_side = false;
    }

    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
        for pf in self.parametric_filters.iter_mut() {
            pf.reset();
        }
    }

    pub fn check_active(&mut self) {
        let std_active = self.gains_db.iter().enumerate().any(|(i, g)| match self.filter_types[i] {
            FilterType::LowPass | FilterType::HighPass | FilterType::Notch { .. } | FilterType::BandPass { .. } => true,
            _ => g.abs() > 0.05,
        });

        let param_active = self.parametric_filters.iter().any(|f| f.active);

        self.has_mid_side = self.parametric_filters.iter().any(|f| {
            f.active && (f.channel == EqChannel::Mid || f.channel == EqChannel::Side)
        });

        self.is_active = std_active || param_active;
    }

    /// Procesa un bloque de muestras estéreo en formato planar con aceleración SIMD
    /// e inversión de bucles (filtro exterior, buffer interior)
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.is_active {
            return;
        }

        let len = left.len().min(right.len());

        // 1. Filtros ISO estándar de 15 bandas (Stereo)
        for filter in self.filters.iter_mut() {
            if filter.active {
                filter.process_buffer(left, right);
            }
        }

        // 2. Filtros paramétricos directos (Stereo, Left, Right)
        for f in self.parametric_filters.iter_mut() {
            if !f.active {
                continue;
            }
            match f.channel {
                EqChannel::Stereo => {
                    for i in 0..len {
                        left[i] = f.state_l.process_sample(&f.coeffs, left[i]);
                        right[i] = f.state_r.process_sample(&f.coeffs, right[i]);
                    }
                }
                EqChannel::Left => {
                    for s in left[..len].iter_mut() {
                        *s = f.state_l.process_sample(&f.coeffs, *s);
                    }
                }
                EqChannel::Right => {
                    for s in right[..len].iter_mut() {
                        *s = f.state_r.process_sample(&f.coeffs, *s);
                    }
                }
                EqChannel::Mid | EqChannel::Side => {}
            }
        }

        // 3. Matriz Mid/Side (M/S): procesamiento ortogonal transparente de fase
        if self.has_mid_side {
            const INV_SQRT2: f32 = 0.70710678;

            // Codificación L/R -> M/S
            for i in 0..len {
                let m = (left[i] + right[i]) * INV_SQRT2;
                let s = (left[i] - right[i]) * INV_SQRT2;
                left[i] = m;
                right[i] = s;
            }

            // Aplicación de filtros Mid (sobre left) y Side (sobre right)
            for f in self.parametric_filters.iter_mut() {
                if !f.active {
                    continue;
                }
                if f.channel == EqChannel::Mid {
                    for s in left[..len].iter_mut() {
                        *s = f.state_l.process_sample(&f.coeffs, *s);
                    }
                } else if f.channel == EqChannel::Side {
                    for s in right[..len].iter_mut() {
                        *s = f.state_r.process_sample(&f.coeffs, *s);
                    }
                }
            }

            // Decodificación M/S -> L/R
            for i in 0..len {
                let m = left[i];
                let s = right[i];
                left[i] = (m + s) * INV_SQRT2;
                right[i] = (m - s) * INV_SQRT2;
            }
        }
    }

    /// Procesa un bloque de muestras mono
    #[inline]
    pub fn process_mono(&mut self, mono: &mut [f32]) {
        if !self.is_active {
            return;
        }

        for filter in self.filters.iter_mut() {
            if filter.active {
                for sample in mono.iter_mut() {
                    *sample = filter.left.process_sample(&filter.coeffs, *sample);
                }
            }
        }

        for f in self.parametric_filters.iter_mut() {
            if f.active && matches!(f.channel, EqChannel::Stereo | EqChannel::Mid | EqChannel::Left) {
                for sample in mono.iter_mut() {
                    *sample = f.state_l.process_sample(&f.coeffs, *sample);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eq_standard_bands_processing() {
        let mut eq = Equalizer::new(48000.0);
        eq.set_band_db(0, 6.0); // 25 Hz boost
        assert!(eq.is_active);

        let mut left = vec![1.0; 128];
        let mut right = vec![1.0; 128];
        eq.process_planar(&mut left, &mut right);

        assert!(left[0] != 1.0 || left[127] != 1.0);
    }

    #[test]
    fn test_parametric_bands() {
        let mut eq = Equalizer::new(48000.0);
        eq.set_parametric_bands(&[
            ParametricBand {
                freq: 1000.0,
                gain_db: 4.0,
                q: 1.5,
                filter_type: "peaking".to_string(),
                channel: EqChannel::Stereo,
            },
        ]);

        assert!(eq.is_active);
        assert!(!eq.has_mid_side);
        assert_eq!(eq.parametric_filters.len(), 1);

        let mut left = vec![0.5; 64];
        let mut right = vec![0.5; 64];
        eq.process_planar(&mut left, &mut right);
        assert!(left[10].is_finite());
    }

    #[test]
    fn test_mid_side_equalization() {
        let mut eq = Equalizer::new(48000.0);
        // Mid boost (dialogue) + Side cut (sub-bass cleanup)
        eq.set_parametric_bands(&[
            ParametricBand {
                freq: 2000.0,
                gain_db: 6.0,
                q: 2.0,
                filter_type: "peaking".to_string(),
                channel: EqChannel::Mid,
            },
            ParametricBand {
                freq: 100.0,
                gain_db: 0.0,
                q: 0.707,
                filter_type: "highpass".to_string(),
                channel: EqChannel::Side,
            },
        ]);

        assert!(eq.is_active);
        assert!(eq.has_mid_side);

        let mut left = vec![1.0; 256];
        let mut right = vec![-1.0; 256]; // Pure Side (M = 0, S != 0)
        eq.process_planar(&mut left, &mut right);

        // HighPass on Side will attenuate low-frequency out-of-phase signal
        assert!(left[255].abs() < 1.0);
        assert!(right[255].abs() < 1.0);
    }

    #[test]
    fn test_dolby_20_bands() {
        let mut eq = Equalizer::new(48000.0);
        let gains = [1.0; 20];
        eq.set_dolby_20_bands(&gains);
        assert_eq!(eq.parametric_filters.len(), 20);
        assert!(eq.is_active);
    }

    #[test]
    fn test_iso_31_bands() {
        let mut eq = Equalizer::new(48000.0);
        let gains = [0.5; 31];
        eq.set_iso_31_bands(&gains);
        assert_eq!(eq.parametric_filters.len(), 31);
        assert!(eq.is_active);
    }
}
