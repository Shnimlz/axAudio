//! Filtros IIR Biquad (Audio EQ Cookbook - Robert Bristow-Johnson)
//!
//! Implementación matemática de alta fidelidad para ecualización en tiempo real.

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

impl BiquadCoefficients {
    /// Calcula los coeficientes biquad para una frecuencia central, ganancia en dB y frecuencia de muestreo
    pub fn compute(filter_type: FilterType, freq_hz: f32, gain_db: f32, sample_rate: f32) -> Self {
        let f0 = freq_hz.clamp(10.0, sample_rate * 0.49);
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f0 / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();

        match filter_type {
            FilterType::Peaking { q } => {
                let alpha = sin_w0 / (2.0 * q.max(0.1));
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

/// Estado del filtro biquad para un solo canal de audio (Direct Form I / Transposed II)
#[derive(Debug, Clone, Default)]
pub struct BiquadState {
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

impl BiquadState {
    #[inline(always)]
    pub fn process_sample(&mut self, coeffs: &BiquadCoefficients, sample: f32) -> f32 {
        let mut out = coeffs.b0 * sample + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
            - coeffs.a1 * self.y1
            - coeffs.a2 * self.y2;

        // Anti-denormal flush (evita números subnormales que ralentizan el hilo de audio y crean siseo)
        if out.abs() < 1e-15 {
            out = 0.0;
        }

        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = out;

        out
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Filtro biquad estéreo completo (mantiene coeficientes y estados L/R)
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
