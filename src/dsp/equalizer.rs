//! Ecualizador gráfico de 15 bandas con precisión de estudio
//!
//! Frecuencias estándar ISO / Lavalink:
//! 25, 40, 63, 100, 160, 250, 400, 630, 1000, 1600, 2500, 4000, 6300, 10000, 16000 Hz.
//!
//! Incorpora calibración de factor de calidad (Q) psicoacústico dependiente de la frecuencia:
//! Q moderado y estabilizado en subgraves para evitar zumbidos y resonancias artificiales,
//! Q quirúrgico en rango vocal/medios y Q suave en agudos para un timbre orgánico y cálido.

use super::biquad::{FilterType, StereoBiquad};
use serde::{Deserialize, Serialize};

pub const EQ_FREQUENCIES: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandSetting {
    pub band: usize,
    #[serde(default)]
    pub gain: Option<f32>,     // Lineal Lavalink (-0.25 a +1.0)
    #[serde(default)]
    pub gain_db: Option<f32>,  // Decibelios directos (-15.0 a +15.0)
    #[serde(default)]
    pub filter_type: Option<String>, // "peaking" (default), "low_shelf", "high_shelf", "low_pass", "high_pass"
}

#[derive(Debug, Clone)]
pub struct Equalizer {
    pub filters: Vec<StereoBiquad>,
    pub filter_types: [FilterType; 15],
    pub gains_db: [f32; 15],
    pub sample_rate: f32,
    pub is_active: bool,
}

impl Equalizer {
    pub fn parse_filter_type(band: usize, s: &str) -> FilterType {
        match s.to_lowercase().replace('_', "").as_str() {
            "lowshelf" | "shelfdown" | "bassshelf" => FilterType::LowShelf,
            "highshelf" | "shelfup" | "trebleshelf" => FilterType::HighShelf,
            "lowpass" | "lp" | "hicut" => FilterType::LowPass,
            "highpass" | "hp" | "lowcut" => FilterType::HighPass,
            _ => FilterType::Peaking { q: get_band_q(band) },
        }
    }

    pub fn new(sample_rate: f32) -> Self {
        let mut filters = Vec::with_capacity(15);
        let mut filter_types = [FilterType::Peaking { q: 1.0 }; 15];
        for (band, &freq) in EQ_FREQUENCIES.iter().enumerate() {
            let ft = FilterType::Peaking { q: get_band_q(band) };
            filter_types[band] = ft;
            filters.push(StereoBiquad::new(
                ft,
                freq,
                0.0,
                sample_rate,
            ));
        }

        Self {
            filters,
            filter_types,
            gains_db: [0.0; 15],
            sample_rate,
            is_active: false,
        }
    }

    /// Adapta los filtros si la tasa de muestreo cambia
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 1.0 {
            self.sample_rate = sample_rate;
            for (i, filter) in self.filters.iter_mut().enumerate() {
                let freq = EQ_FREQUENCIES[i];
                filter.update(
                    self.filter_types[i],
                    freq,
                    self.gains_db[i],
                    sample_rate,
                );
            }
        }
    }

    /// Configura una banda específica (0 a 14) con ganancia en dB (tipo Peaking por defecto)
    pub fn set_band_db(&mut self, band: usize, gain_db: f32) {
        if band < 15 {
            let clamped = gain_db.clamp(-15.0, 15.0);
            self.gains_db[band] = clamped;
            let freq = EQ_FREQUENCIES[band];
            self.filters[band].update(
                self.filter_types[band],
                freq,
                clamped,
                self.sample_rate,
            );
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
            self.filters[band].update(
                ft,
                freq,
                clamped,
                self.sample_rate,
            );
            self.check_active();
        }
    }

    /// Configura una banda usando la escala Lavalink (-0.25 a +1.0) convirtiendo a dB
    pub fn set_band_lavalink(&mut self, band: usize, gain_linear: f32) {
        if band < 15 {
            // Lavalink: gain = 0.0 -> 0 dB; gain = 0.25 -> +3 dB; gain = -0.25 -> -12 dB
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
                if linear >= 0.0 { linear * 12.0 } else { linear * 48.0 }
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

    /// Calcula la ganancia pico real de la curva combinada de los 15 filtros
    /// para pre-amplificación exacta contra saturación
    pub fn compute_max_peak_gain_db(&self) -> f32 {
        let active_filters: Vec<&StereoBiquad> = self.filters.iter().filter(|f| f.active).collect();
        if active_filters.is_empty() {
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
            let sum_db: f64 = active_filters.iter().map(|filter| filter.coeffs.mag_db(w)).sum();
            if sum_db > max_gain_db {
                max_gain_db = sum_db;
            }
        }

        max_gain_db as f32
    }

    /// Restablece todas las bandas a plano (0 dB) y tipo Peaking
    #[allow(dead_code)]
    pub fn reset_flat(&mut self) {
        self.gains_db = [0.0; 15];
        for (i, filter) in self.filters.iter_mut().enumerate() {
            let ft = FilterType::Peaking { q: get_band_q(i) };
            self.filter_types[i] = ft;
            filter.update(
                ft,
                EQ_FREQUENCIES[i],
                0.0,
                self.sample_rate,
            );
            filter.reset();
        }
        self.is_active = false;
    }

    pub fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
    }

    pub fn check_active(&mut self) {
        self.is_active = self.gains_db.iter().enumerate().any(|(i, g)| {
            match self.filter_types[i] {
                FilterType::LowPass | FilterType::HighPass => true,
                _ => g.abs() > 0.05,
            }
        });
    }

    /// Procesa un bloque de muestras estéreo en formato planar
    /// Inversión de bucles (filtro exterior, buffer interior) para vectorización SIMD nativa
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.is_active {
            return;
        }

        for filter in self.filters.iter_mut() {
            if filter.active {
                filter.process_buffer(left, right);
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
    }
}
