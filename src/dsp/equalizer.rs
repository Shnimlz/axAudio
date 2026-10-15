//! Ecualizador gráfico de 15 bandas con precisión de estudio
//!
//! Frecuencias estándar ISO / Lavalink:
//! 25, 40, 63, 100, 160, 250, 400, 630, 1000, 1600, 2500, 4000, 6300, 10000, 16000 Hz.

use super::biquad::{FilterType, StereoBiquad};
use serde::{Deserialize, Serialize};

pub const EQ_FREQUENCIES: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandSetting {
    pub band: usize,
    pub gain: f32, // en dB (-15.0 a +15.0) o lineal Lavalink (-0.25 a +1.0)
}

#[derive(Debug, Clone)]
pub struct Equalizer {
    pub filters: Vec<StereoBiquad>,
    pub gains_db: [f32; 15],
    pub sample_rate: f32,
    pub is_active: bool,
}

impl Equalizer {
    pub fn new(sample_rate: f32) -> Self {
        let mut filters = Vec::with_capacity(15);
        for &freq in &EQ_FREQUENCIES {
            filters.push(StereoBiquad::new(
                FilterType::Peaking { q: 1.1 },
                freq,
                0.0,
                sample_rate,
            ));
        }

        Self {
            filters,
            gains_db: [0.0; 15],
            sample_rate,
            is_active: false,
        }
    }

    /// Configura una banda específica (0 a 14) con ganancia en dB
    pub fn set_band_db(&mut self, band: usize, gain_db: f32) {
        if band < 15 {
            let clamped = gain_db.clamp(-15.0, 15.0);
            self.gains_db[band] = clamped;
            let freq = EQ_FREQUENCIES[band];
            self.filters[band].update(FilterType::Peaking { q: 1.1 }, freq, clamped, self.sample_rate);
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

    /// Aplica una lista de bandas de forma atómica
    pub fn set_bands(&mut self, bands: &[BandSetting]) {
        for b in bands {
            // Si el valor está entre -0.5 y 1.0, interpretamos como escala Lavalink; si es > 1.0 o < -0.5, como dB directos
            if b.gain.abs() <= 1.0 && b.gain != 0.0 {
                self.set_band_lavalink(b.band, b.gain);
            } else {
                self.set_band_db(b.band, b.gain);
            }
        }
    }

    /// Restablece todas las bandas a plano (0 dB)
    pub fn reset_flat(&mut self) {
        self.gains_db = [0.0; 15];
        for (i, filter) in self.filters.iter_mut().enumerate() {
            filter.update(
                FilterType::Peaking { q: 1.1 },
                EQ_FREQUENCIES[i],
                0.0,
                self.sample_rate,
            );
            filter.reset();
        }
        self.is_active = false;
    }

    fn check_active(&mut self) {
        self.is_active = self.gains_db.iter().any(|g| g.abs() > 0.05);
    }

    /// Procesa un bloque de muestras estéreo en formato planar (Left y Right slices)
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.is_active {
            return;
        }

        let len = left.len().min(right.len());
        for i in 0..len {
            let mut l = left[i];
            let mut r = right[i];

            for filter in &mut self.filters {
                if filter.active {
                    let (fl, fr) = filter.process_pair(l, r);
                    l = fl;
                    r = fr;
                }
            }

            left[i] = l;
            right[i] = r;
        }
    }

    /// Procesa un bloque de muestras mono
    #[inline]
    pub fn process_mono(&mut self, mono: &mut [f32]) {
        if !self.is_active {
            return;
        }

        for sample in mono.iter_mut() {
            let mut s = *sample;
            for filter in &mut self.filters {
                if filter.active {
                    let (fl, _) = filter.process_pair(s, s);
                    s = fl;
                }
            }
            *sample = s;
        }
    }
}
