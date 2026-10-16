//! Filtros IIR Biquad (Audio EQ Cookbook - Robert Bristow-Johnson)
//!
//! Implementación matemática de alta fidelidad con topología Transposed Direct Form II (TDF-II).
//! Incluye estabilización de factor Q en subgraves y bypass transparente de ganancia cero.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    Peaking { q: f32 },
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

#[derive(Debug, Clone)]
pub struct BiquadCoefficients {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for BiquadCoefficients {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl BiquadCoefficients {
    /// Calcula los coeficientes biquad para una frecuencia central, ganancia en dB y frecuencia de muestreo
    pub fn compute(filter_type: FilterType, freq_hz: f32, gain_db: f32, sample_rate: f32) -> Self {
        // Zero-gain bypass: si la ganancia es prácticamente nula en un filtro peaking o shelf,
        // devolvemos ganancia unitaria directa sin alterar la señal
        if matches!(filter_type, FilterType::Peaking { .. } | FilterType::LowShelf | FilterType::HighShelf)
            && gain_db.abs() < 0.01
        {
            return Self::default();
        }

        let f0 = freq_hz.clamp(10.0, sample_rate * 0.49);
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f0 / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();

        match filter_type {
            FilterType::Peaking { q } => {
                // Estabilización de Q en graves:
                // En frecuencias subgraves (< 60 Hz), un Q alto causa inestabilidad numérica
                // severa y siseo/estática en coma flotante de 32 bits. Tapering suave hacia 0.8 en 20 Hz.
                let effective_q = if f0 < 60.0 {
                    let max_q = (f0 - 20.0).max(0.0) * ((q - 0.8) / 40.0) + 0.8;
                    q.min(max_q).max(0.2)
                } else {
                    q.max(0.1)
                };

                let alpha = sin_w0 / (2.0 * effective_q);
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha / a;
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_w0;
                let b2 = 1.0 - alpha * a;

                Self {
                    b0: b0 / a0,
                    b1: b1 / a0,
                    b2: b2 / a0,
                    a1: a1 / a0,
                    a2: a2 / a0,
                }
            }
            FilterType::LowShelf => {
                let s = 1.0; // slope
                let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

                let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);

                Self {
                    b0: b0 / a0,
                    b1: b1 / a0,
                    b2: b2 / a0,
                    a1: a1 / a0,
                    a2: a2 / a0,
                }
            }
            FilterType::HighShelf => {
                let s = 1.0;
                let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

                let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);

                Self {
                    b0: b0 / a0,
                    b1: b1 / a0,
                    b2: b2 / a0,
                    a1: a1 / a0,
                    a2: a2 / a0,
                }
            }
            FilterType::LowPass => {
                let q = 0.7071; // Butterworth Q
                let alpha = sin_w0 / (2.0 * q);
                let a0 = 1.0 + alpha;
                let b1 = 1.0 - cos_w0;
                let b0 = b1 / 2.0;
                let b2 = b0;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;

                Self {
                    b0: b0 / a0,
                    b1: b1 / a0,
                    b2: b2 / a0,
                    a1: a1 / a0,
                    a2: a2 / a0,
                }
            }
            FilterType::HighPass => {
                let q = 0.7071;
                let alpha = sin_w0 / (2.0 * q);
                let a0 = 1.0 + alpha;
                let b1 = -(1.0 + cos_w0);
                let b0 = (1.0 + cos_w0) / 2.0;
                let b2 = b0;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;

                Self {
                    b0: b0 / a0,
                    b1: b1 / a0,
                    b2: b2 / a0,
                    a1: a1 / a0,
                    a2: a2 / a0,
                }
            }
        }
    }
}

/// Estado del filtro biquad en Transposed Direct Form II (TDF-II)
///
/// A diferencia de Direct Form I (que mantiene x1, x2, y1, y2), TDF-II solo almacena dos
/// variables de estado integradas (s1, s2). Esta topología reduce drásticamente el ruido
/// de cuantización y cancelación en f32, eliminando ruidos de fondo y estática en frecuencias bajas.
#[derive(Debug, Clone, Default)]
pub struct BiquadState {
    pub s1: f32,
    pub s2: f32,
}

impl BiquadState {
    #[inline(always)]
    pub fn process_sample(&mut self, coeffs: &BiquadCoefficients, sample: f32) -> f32 {
        // Ecuaciones Transposed Direct Form II:
        // y[n] = b0 * x[n] + s1
        // s1   = b1 * x[n] - a1 * y[n] + s2
        // s2   = b2 * x[n] - a2 * y[n]
        let out = coeffs.b0 * sample + self.s1;

        // Anti-denormal flush con umbral microscópico para evitar sobrecarga en CPU
        let clean_out = if out.abs() < 1e-15 { 0.0 } else { out };

        self.s1 = coeffs.b1 * sample - coeffs.a1 * clean_out + self.s2;
        self.s2 = coeffs.b2 * sample - coeffs.a2 * clean_out;

        clean_out
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Filtro biquad estéreo completo (mantiene coeficientes y estados L/R en TDF-II)
#[derive(Debug, Clone)]
pub struct StereoBiquad {
    pub coeffs: BiquadCoefficients,
    pub left: BiquadState,
    pub right: BiquadState,
    pub active: bool,
}

impl StereoBiquad {
    pub fn new(filter_type: FilterType, freq_hz: f32, gain_db: f32, sample_rate: f32) -> Self {
        let active = matches!(filter_type, FilterType::LowPass | FilterType::HighPass) || gain_db.abs() > 0.01;
        Self {
            coeffs: BiquadCoefficients::compute(filter_type, freq_hz, gain_db, sample_rate),
            left: BiquadState::default(),
            right: BiquadState::default(),
            active,
        }
    }

    pub fn update(&mut self, filter_type: FilterType, freq_hz: f32, gain_db: f32, sample_rate: f32) {
        self.coeffs = BiquadCoefficients::compute(filter_type, freq_hz, gain_db, sample_rate);
        self.active = matches!(filter_type, FilterType::LowPass | FilterType::HighPass) || gain_db.abs() > 0.01;
    }

    #[inline(always)]
    pub fn process_pair(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.active {
            return (l, r);
        }
        (
            self.left.process_sample(&self.coeffs, l),
            self.right.process_sample(&self.coeffs, r),
        )
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_gain_bypass() {
        let mut filter = StereoBiquad::new(FilterType::Peaking { q: 1.0 }, 1000.0, 0.0, 48000.0);
        assert!(!filter.active);
        let (l, r) = filter.process_pair(0.5, -0.25);
        assert_eq!(l, 0.5);
        assert_eq!(r, -0.25);
    }

    #[test]
    fn test_sub_bass_stability() {
        // Test 25 Hz sub-bass filter with aggressive boost
        let mut filter = StereoBiquad::new(FilterType::Peaking { q: 1.8 }, 25.0, 12.0, 48000.0);
        assert!(filter.active);

        // Process impulse followed by silence for 1000 samples
        let (first_l, first_r) = filter.process_pair(1.0, 1.0);
        assert!(first_l.is_finite());
        assert!(first_r.is_finite());

        for _ in 0..1000 {
            let (l, r) = filter.process_pair(0.0, 0.0);
            assert!(l.is_finite());
            assert!(r.is_finite());
            assert!(l.abs() < 5.0);
            assert!(r.abs() < 5.0);
        }
    }
}

