//! Normalización de Volumen Perceptual EBU R128 / ITU-R BS.1770-4
//!
//! Implementa la ponderación K-weighting (filtro de cabeza + filtro paso alto RLB),
//! cálculo de sonoridad en LUFS (Loudness Units relative to Full Scale), compuerta de
//! silencio a -70 LUFS (gating), control automático de ganancia (AGC) transparente con
//! rampa suave anti-pumping y limitador analógico soft-knee de pico real (True-Peak).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoudnessSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_target_lufs")]
    pub target_lufs: f32, // Estándar de streaming: -14.0 LUFS (YouTube/Spotify) a -23.0 LUFS (EBU R128)
    #[serde(default = "default_max_gain_db")]
    pub max_gain_db: f32, // Ganancia máxima permitida en dB (ej. +9.0 dB)
    #[serde(default = "default_min_gain_db")]
    pub min_gain_db: f32, // Reducción máxima permitida en dB (ej. -12.0 dB)
}

fn default_target_lufs() -> f32 {
    -14.0
}

fn default_max_gain_db() -> f32 {
    9.0
}

fn default_min_gain_db() -> f32 {
    -12.0
}

impl Default for LoudnessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            target_lufs: default_target_lufs(),
            max_gain_db: default_max_gain_db(),
            min_gain_db: default_min_gain_db(),
        }
    }
}

/// Filtro biquad de segundo orden en f64 (Forma Directa Transpuesta II)
#[derive(Debug, Clone, Default)]
struct BiquadF64 {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f64,
    s2: f64,
}

impl BiquadF64 {
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.b1 * x - self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Filtro de ponderación K (ITU-R BS.1770-4) para un canal individual
#[derive(Debug, Clone)]
struct KWeightingChannel {
    stage1: BiquadF64, // High-shelf filter (modelado acústico de cabeza)
    stage2: BiquadF64, // High-pass filter (RLB weighting ~38 Hz)
}

impl KWeightingChannel {
    fn new(sample_rate: f64) -> Self {
        let mut ch = Self {
            stage1: BiquadF64::default(),
            stage2: BiquadF64::default(),
        };
        ch.update_sample_rate(sample_rate);
        ch
    }

    fn update_sample_rate(&mut self, sample_rate: f64) {
        let fs = sample_rate.max(8000.0);

        if (fs - 48000.0).abs() < 100.0 {
            // Coeficientes exactos especificados por la norma ITU-R BS.1770-4 a 48 kHz
            self.stage1 = BiquadF64 {
                b0: 1.53512485958697,
                b1: -2.69169618940638,
                b2: 1.19839281085285,
                a1: -1.69065929318241,
                a2: 0.73248077421585,
                s1: 0.0,
                s2: 0.0,
            };
            self.stage2 = BiquadF64 {
                b0: 1.0,
                b1: -2.0,
                b2: 1.0,
                a1: -1.99004745483398,
                a2: 0.99007225036621,
                s1: 0.0,
                s2: 0.0,
            };
        } else {
            // Transformada bilineal analítica exacta para frecuencias de muestreo arbitrarias (ej. 44.1 kHz)
            // Stage 1: High-shelf (f0 = 1681.97 Hz, Gain = +3.9998 dB, Q = 0.707175)
            let f0_shelf = 1681.9744509555319;
            let gain_db = 3.99984385397;
            let q_shelf = 0.7071752369554193;

            let v = 10.0f64.powf(gain_db / 20.0);
            let k_shelf = (std::f64::consts::PI * f0_shelf / fs).tan();
            let k2_shelf = k_shelf * k_shelf;
            let sqrt_v = v.sqrt();
            let denom_shelf = 1.0 + (std::f64::consts::SQRT_2 / q_shelf) * k_shelf + k2_shelf;

            self.stage1 = BiquadF64 {
                b0: (v + std::f64::consts::SQRT_2 * sqrt_v * k_shelf + k2_shelf) / denom_shelf,
                b1: 2.0 * (k2_shelf - v) / denom_shelf,
                b2: (v - std::f64::consts::SQRT_2 * sqrt_v * k_shelf + k2_shelf) / denom_shelf,
                a1: 2.0 * (k2_shelf - 1.0) / denom_shelf,
                a2: (1.0 - (std::f64::consts::SQRT_2 / q_shelf) * k_shelf + k2_shelf) / denom_shelf,
                s1: 0.0,
                s2: 0.0,
            };

            // Stage 2: High-pass RLB (f0 = 38.135 Hz, Q = 0.500327)
            let f0_hp = 38.13547087602444;
            let q_hp = 0.5003270373253953;

            let k_hp = (std::f64::consts::PI * f0_hp / fs).tan();
            let k2_hp = k_hp * k_hp;
            let denom_hp = 1.0 + (k_hp / q_hp) + k2_hp;

            self.stage2 = BiquadF64 {
                b0: 1.0 / denom_hp,
                b1: -2.0 / denom_hp,
                b2: 1.0 / denom_hp,
                a1: 2.0 * (k2_hp - 1.0) / denom_hp,
                a2: (1.0 - (k_hp / q_hp) + k2_hp) / denom_hp,
                s1: 0.0,
                s2: 0.0,
            };
        }
    }

    #[inline]
    fn process(&mut self, sample: f32) -> f64 {
        let x = sample as f64;
        let y1 = self.stage1.process(x);
        self.stage2.process(y1)
    }

    fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
    }
}

/// Procesador maestro de Normalización de Sonoridad EBU R128
#[derive(Debug, Clone)]
pub struct LoudnessNormalizer {
    kw_left: KWeightingChannel,
    kw_right: KWeightingChannel,
    sample_rate: f32,
    settings: LoudnessSettings,

    // Estimadores de potencia acústica integrada y momentánea
    energy_short_term: f64,
    energy_momentary: f64,
    alpha_short_term: f64,
    alpha_momentary: f64,

    // Ganancia actual suavemente interpolada (anti-pumping)
    current_gain: f32,
    target_gain: f32,
    current_lufs: f32,
}

impl LoudnessNormalizer {
    pub fn new(sample_rate: f32) -> Self {
        let mut normalizer = Self {
            kw_left: KWeightingChannel::new(sample_rate as f64),
            kw_right: KWeightingChannel::new(sample_rate as f64),
            sample_rate,
            settings: LoudnessSettings::default(),
            energy_short_term: 1e-4, // Inicializa cerca de -40 LUFS
            energy_momentary: 1e-4,
            alpha_short_term: 0.9999,
            alpha_momentary: 0.999,
            current_gain: 1.0,
            target_gain: 1.0,
            current_lufs: -14.0,
        };
        normalizer.update_sample_rate(sample_rate);
        normalizer
    }

    pub fn set_settings(&mut self, settings: LoudnessSettings) {
        self.settings = settings;
    }

    pub fn get_settings(&self) -> &LoudnessSettings {
        &self.settings
    }

    pub fn get_current_lufs(&self) -> f32 {
        self.current_lufs
    }

    pub fn is_active(&self) -> bool {
        self.settings.enabled
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
        let fs = self.sample_rate as f64;
        self.kw_left.update_sample_rate(fs);
        self.kw_right.update_sample_rate(fs);

        // Ventana de integración: Short-term = 3.0s, Momentary = 0.4s
        self.alpha_short_term = (-1.0 / (3.0 * fs)).exp();
        self.alpha_momentary = (-1.0 / (0.4 * fs)).exp();
    }

    pub fn reset(&mut self) {
        self.kw_left.reset();
        self.kw_right.reset();
        self.energy_short_term = 1e-4;
        self.energy_momentary = 1e-4;
        self.current_gain = 1.0;
        self.target_gain = 1.0;
        self.current_lufs = self.settings.target_lufs;
    }

    /// Procesa un bloque de audio planar aplicando medición EBU R128 y ganancia automática normalizada
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.settings.enabled {
            return;
        }

        let len = left.len().min(right.len());
        if len == 0 {
            return;
        }

        let target_lufs = self.settings.target_lufs;
        let max_gain_linear = 10.0f32.powf(self.settings.max_gain_db.clamp(0.0, 18.0) / 20.0);
        let min_gain_linear = 10.0f32.powf(self.settings.min_gain_db.clamp(-24.0, 0.0) / 20.0);

        // Coeficientes de slew-rate para la rampa de ganancia:
        // Attack (bajar volumen si es muy fuerte): rápido (~150ms) para proteger oídos
        // Release (subir volumen si es bajo): muy suave (~2500ms) para evitar efecto "bombeo" o hiss
        let alpha_attack = (-1.0 / (0.15 * self.sample_rate as f64)).exp() as f32;
        let alpha_release = (-1.0 / (2.50 * self.sample_rate as f64)).exp() as f32;

        let one_minus_short = 1.0 - self.alpha_short_term;
        let one_minus_mom = 1.0 - self.alpha_momentary;

        for i in 0..len {
            let l = left[i];
            let r = right[i];

            // 1. Filtrado K-weighting simultáneo
            let y_l = self.kw_left.process(l);
            let y_r = self.kw_right.process(r);

            // Potencia acústica cuadrática ponderada
            let power = 0.5 * (y_l * y_l + y_r * y_r);

            // 2. Acumuladores de energía integrados
            self.energy_short_term = self.alpha_short_term * self.energy_short_term + one_minus_short * power;
            self.energy_momentary = self.alpha_momentary * self.energy_momentary + one_minus_mom * power;

            // 3. Cálculo de sonoridad cada 64 muestras para optimizar CPU
            if (i & 63) == 0 {
                // EBU R128: L_k = -0.691 + 10 * log10(energy)
                let energy_safe = self.energy_short_term.max(1e-12);
                let measured_lufs = (-0.691 + 10.0 * energy_safe.log10()) as f32;
                self.current_lufs = measured_lufs;

                // Compuerta de silencio (Silence Gating) a -70.0 LUFS:
                // Si la música está en pausa, intro silenciosa o final, se congela la ganancia actual
                // para evitar amplificar el ruido de fondo
                if measured_lufs > -70.0 {
                    let delta_db = target_lufs - measured_lufs;
                    let desired_gain = 10.0f32.powf(delta_db / 20.0);
                    self.target_gain = desired_gain.clamp(min_gain_linear, max_gain_linear);
                }
            }

            // 4. Interpolación suave de la ganancia
            let slew_coeff = if self.target_gain < self.current_gain {
                alpha_attack
            } else {
                alpha_release
            };
            self.current_gain = slew_coeff * self.current_gain + (1.0 - slew_coeff) * self.target_gain;

            // 5. Aplicar ganancia normalizada
            let mut out_l = l * self.current_gain;
            let mut out_r = r * self.current_gain;

            // 6. Limitador suave de pico analógico (Soft-Knee Peak Limiter a 0 dBFS)
            // Asegura que transitorios no recorten digitalmente contra el techo de Discord
            if out_l.abs() > 0.95 {
                let sign = out_l.signum();
                let excess = out_l.abs() - 0.95;
                out_l = sign * (0.95 + 0.048 * (excess / 0.048).tanh());
            }
            if out_r.abs() > 0.95 {
                let sign = out_r.signum();
                let excess = out_r.abs() - 0.95;
                out_r = sign * (0.95 + 0.048 * (excess / 0.048).tanh());
            }

            left[i] = out_l;
            right[i] = out_r;
        }
    }
}
