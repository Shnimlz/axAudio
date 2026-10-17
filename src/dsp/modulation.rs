//! Efectos de Modulación y Espacialización Dinámica (8D Audio, Tremolo, Vibrato)
//!
//! - Rotación 8D: Panoramización orbital continua con ley de potencia constante (3 dB Pan Law)
//! - Tremolo: Modulación de amplitud periódica con oscilador de baja frecuencia (LFO)
//! - Vibrato: Modulación de tono mediante línea de retardo fraccional e interpolación lineal

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationSettings {
    #[serde(default)]
    pub rotation: bool,
    #[serde(default = "default_rotation_speed")]
    pub rotation_speed_hz: f32, // Hz (0.02 a 2.0 Hz, default 0.15 Hz = ~6.6s por órbita completa)
    #[serde(default)]
    pub tremolo: bool,
    #[serde(default = "default_tremolo_freq")]
    pub tremolo_freq_hz: f32,   // Hz (0.5 a 20.0 Hz, default 4.0 Hz)
    #[serde(default = "default_tremolo_depth")]
    pub tremolo_depth: f32,     // 0.0 a 1.0 (default 0.5)
    #[serde(default)]
    pub vibrato: bool,
    #[serde(default = "default_vibrato_freq")]
    pub vibrato_freq_hz: f32,   // Hz (0.5 a 14.0 Hz, default 5.0 Hz)
    #[serde(default = "default_vibrato_depth")]
    pub vibrato_depth: f32,     // 0.0 a 1.0 (default 0.3)
}

fn default_rotation_speed() -> f32 {
    0.15
}
fn default_tremolo_freq() -> f32 {
    4.0
}
fn default_tremolo_depth() -> f32 {
    0.5
}
fn default_vibrato_freq() -> f32 {
    5.0
}
fn default_vibrato_depth() -> f32 {
    0.3
}

impl Default for ModulationSettings {
    fn default() -> Self {
        Self {
            rotation: false,
            rotation_speed_hz: default_rotation_speed(),
            tremolo: false,
            tremolo_freq_hz: default_tremolo_freq(),
            tremolo_depth: default_tremolo_depth(),
            vibrato: false,
            vibrato_freq_hz: default_vibrato_freq(),
            vibrato_depth: default_vibrato_depth(),
        }
    }
}

impl ModulationSettings {
    pub fn is_active(&self) -> bool {
        self.rotation || self.tremolo || self.vibrato
    }
}

#[derive(Debug, Clone)]
pub struct RotationProcessor {
    phase: f32,
    sample_rate: f32,
    speed_hz: f32,
    active: bool,
}

impl RotationProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            sample_rate,
            speed_hz: default_rotation_speed(),
            active: false,
        }
    }

    pub fn set_speed(&mut self, speed_hz: f32) {
        self.speed_hz = speed_hz.clamp(0.01, 5.0);
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        if !active {
            self.phase = 0.0;
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active {
            return;
        }

        let phase_inc = (2.0 * PI * self.speed_hz) / self.sample_rate;
        let len = left.len().min(right.len());

        for i in 0..len {
            // Paneo orbital continuo: pan oscila armónicamente entre -1.0 y +1.0
            let pan = self.phase.sin();
            // Ángulo de paneo en rango [0, PI / 2] para ley de potencia constante (Constant Power 3dB Pan Law)
            let angle = (pan + 1.0) * (PI * 0.25);
            let gain_l = angle.cos();
            let gain_r = angle.sin();

            // Mapeo espacial con sangrado sutil cruzado para realismo psicoacústico
            let l = left[i];
            let r = right[i];
            left[i] = l * gain_l + r * (1.0 - gain_r) * 0.18;
            right[i] = r * gain_r + l * (1.0 - gain_l) * 0.18;

            self.phase += phase_inc;
            if self.phase >= 2.0 * PI {
                self.phase -= 2.0 * PI;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TremoloProcessor {
    phase: f32,
    sample_rate: f32,
    freq_hz: f32,
    depth: f32,
    active: bool,
}

impl TremoloProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            sample_rate,
            freq_hz: default_tremolo_freq(),
            depth: default_tremolo_depth(),
            active: false,
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn set_params(&mut self, freq_hz: f32, depth: f32, active: bool) {
        self.freq_hz = freq_hz.clamp(0.2, 30.0);
        self.depth = depth.clamp(0.0, 1.0);
        self.active = active;
        if !active {
            self.phase = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active || self.depth <= 0.001 {
            return;
        }

        let phase_inc = (2.0 * PI * self.freq_hz) / self.sample_rate;
        let len = left.len().min(right.len());

        for i in 0..len {
            let lfo = 0.5 * (1.0 + self.phase.sin());
            let mod_gain = 1.0 - self.depth * (1.0 - lfo);

            left[i] *= mod_gain;
            right[i] *= mod_gain;

            self.phase += phase_inc;
            if self.phase >= 2.0 * PI {
                self.phase -= 2.0 * PI;
            }
        }
    }
}

const VIBRATO_BUFFER_SIZE: usize = 1024;
const VIBRATO_BUFFER_MASK: usize = VIBRATO_BUFFER_SIZE - 1;

#[derive(Debug, Clone)]
pub struct VibratoProcessor {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_idx: usize,
    phase: f32,
    sample_rate: f32,
    freq_hz: f32,
    depth: f32,
    active: bool,
}

impl VibratoProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            buf_l: vec![0.0; VIBRATO_BUFFER_SIZE],
            buf_r: vec![0.0; VIBRATO_BUFFER_SIZE],
            write_idx: 0,
            phase: 0.0,
            sample_rate,
            freq_hz: default_vibrato_freq(),
            depth: default_vibrato_depth(),
            active: false,
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn set_params(&mut self, freq_hz: f32, depth: f32, active: bool) {
        self.freq_hz = freq_hz.clamp(0.2, 20.0);
        self.depth = depth.clamp(0.0, 1.0);
        self.active = active;
        if !active {
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write_idx = 0;
        self.phase = 0.0;
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active || self.depth <= 0.001 {
            return;
        }

        let phase_inc = (2.0 * PI * self.freq_hz) / self.sample_rate;
        // Modulación de retardo máxima: ~3 milisegundos
        let max_delay_samples = (0.003 * self.sample_rate).clamp(10.0, 250.0);
        let center_delay = max_delay_samples + 2.0;
        let excursion = max_delay_samples * self.depth;
        let len = left.len().min(right.len());

        for i in 0..len {
            self.buf_l[self.write_idx] = left[i];
            self.buf_r[self.write_idx] = right[i];

            let delay_samples = center_delay + excursion * self.phase.sin();
            let mut read_pos = self.write_idx as f32 - delay_samples;
            while read_pos < 0.0 {
                read_pos += VIBRATO_BUFFER_SIZE as f32;
            }

            let idx0 = (read_pos as usize) & VIBRATO_BUFFER_MASK;
            let idx1 = (idx0 + 1) & VIBRATO_BUFFER_MASK;
            let frac = read_pos - (read_pos as usize as f32);

            left[i] = self.buf_l[idx0] * (1.0 - frac) + self.buf_l[idx1] * frac;
            right[i] = self.buf_r[idx0] * (1.0 - frac) + self.buf_r[idx1] * frac;

            self.write_idx = (self.write_idx + 1) & VIBRATO_BUFFER_MASK;
            self.phase += phase_inc;
            if self.phase >= 2.0 * PI {
                self.phase -= 2.0 * PI;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModulationProcessor {
    pub rotation: RotationProcessor,
    pub tremolo: TremoloProcessor,
    pub vibrato: VibratoProcessor,
    pub settings: ModulationSettings,
}

impl ModulationProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            rotation: RotationProcessor::new(sample_rate),
            tremolo: TremoloProcessor::new(sample_rate),
            vibrato: VibratoProcessor::new(sample_rate),
            settings: ModulationSettings::default(),
        }
    }

    pub fn set_settings(&mut self, settings: ModulationSettings) {
        self.rotation.set_speed(settings.rotation_speed_hz);
        self.rotation.set_active(settings.rotation);

        self.tremolo.set_params(settings.tremolo_freq_hz, settings.tremolo_depth, settings.tremolo);
        self.vibrato.set_params(settings.vibrato_freq_hz, settings.vibrato_depth, settings.vibrato);

        self.settings = settings;
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.rotation.update_sample_rate(sample_rate);
        self.tremolo.update_sample_rate(sample_rate);
        self.vibrato.update_sample_rate(sample_rate);
    }

    pub fn reset(&mut self) {
        self.rotation.reset();
        self.tremolo.reset();
        self.vibrato.reset();
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.settings.is_active()
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.is_active() {
            return;
        }

        // 1. Vibrato (modulación de tono sobre señal limpia)
        self.vibrato.process_planar(left, right);

        // 2. Tremolo (modulación periódica de amplitud)
        self.tremolo.process_planar(left, right);

        // 3. Rotación 8D (paneo orbital en espacio acústico)
        self.rotation.process_planar(left, right);
    }
}
