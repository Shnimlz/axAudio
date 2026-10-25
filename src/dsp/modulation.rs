//! Efectos de Modulación y Espacialización Dinámica
//!
//! - Rotación 8D: Panoramización orbital continua con ley de potencia constante (3 dB Pan Law)
//! - Tremolo: Modulación de amplitud periódica con oscilador de baja frecuencia (LFO)
//! - Vibrato: Modulación de tono mediante línea de retardo fraccional e interpolación lineal
//! - Flanger: Retroalimentación de retardo corto (0-5 ms) con LFO → "alas de avión" clásico
//! - Chorus: Retardo medio (5-20 ms) con LFO por canal desfasado → ensanchamiento orgánico
//! - Phaser: Red de biquads paso-todos en cascada modulado por LFO → barrido de fase (360° sweep)

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
    #[serde(default)]
    pub flanger: bool,
    #[serde(default = "default_flanger_freq")]
    pub flanger_freq_hz: f32,   // Hz del LFO (0.05 a 8.0, default 0.4)
    #[serde(default = "default_flanger_depth")]
    pub flanger_depth: f32,     // 0.0 a 1.0 (default 0.6; profundidad del barrido de retardo)
    #[serde(default = "default_flanger_feedback")]
    pub flanger_feedback: f32,  // -0.95 a 0.95 (default 0.4; retroalimentación "sweep")
    #[serde(default)]
    pub chorus: bool,
    #[serde(default = "default_chorus_freq")]
    pub chorus_freq_hz: f32,    // Hz del LFO (0.1 a 4.0, default 0.8)
    #[serde(default = "default_chorus_depth")]
    pub chorus_depth: f32,      // 0.0 a 1.0 (default 0.5)
    #[serde(default = "default_chorus_mix")]
    pub chorus_mix: f32,        // 0.0 a 1.0 (default 0.5; mezcla húmeda)
    #[serde(default)]
    pub phaser: bool,
    #[serde(default = "default_phaser_freq")]
    pub phaser_freq_hz: f32,    // Hz del LFO (0.02 a 5.0, default 0.2; barrido lento tipo Moog)
    #[serde(default = "default_phaser_depth")]
    pub phaser_depth: f32,      // 0.0 a 1.0 (default 0.5; profundidad del barrido 360°)
    #[serde(default = "default_phaser_mix")]
    pub phaser_mix: f32,        // 0.0 a 1.0 (default 0.5)
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
fn default_flanger_freq() -> f32 {
    0.4
}
fn default_flanger_depth() -> f32 {
    0.6
}
fn default_flanger_feedback() -> f32 {
    0.4
}
fn default_chorus_freq() -> f32 {
    0.8
}
fn default_chorus_depth() -> f32 {
    0.5
}
fn default_chorus_mix() -> f32 {
    0.5
}
fn default_phaser_freq() -> f32 {
    0.2
}
fn default_phaser_depth() -> f32 {
    0.5
}
fn default_phaser_mix() -> f32 {
    0.5
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
            flanger: false,
            flanger_freq_hz: default_flanger_freq(),
            flanger_depth: default_flanger_depth(),
            flanger_feedback: default_flanger_feedback(),
            chorus: false,
            chorus_freq_hz: default_chorus_freq(),
            chorus_depth: default_chorus_depth(),
            chorus_mix: default_chorus_mix(),
            phaser: false,
            phaser_freq_hz: default_phaser_freq(),
            phaser_depth: default_phaser_depth(),
            phaser_mix: default_phaser_mix(),
        }
    }
}

impl ModulationSettings {
    pub fn is_active(&self) -> bool {
        self.rotation
            || self.tremolo
            || self.vibrato
            || self.flanger
            || self.chorus
            || self.phaser
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

/// Anillo de retardo fraccional reutilizable por Flanger y Chorus (estéreo)
#[derive(Debug, Clone)]
struct StereoDelayRing {
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    write: usize,
}

impl StereoDelayRing {
    fn new(size: usize) -> Self {
        Self {
            ring_l: vec![0.0; size],
            ring_r: vec![0.0; size],
            write: 0,
        }
    }

    fn read(&self, delay: f32, size: usize) -> (f32, f32) {
        let mask = size - 1;
        let mut pl = self.write as f32 - delay;
        let mut pr = pl;
        while pl < 0.0 {
            pl += size as f32;
        }
        while pr < 0.0 {
            pr += size as f32;
        }
        let i0l = (pl as usize) & mask;
        let i1l = (i0l + 1) & mask;
        let i0r = (pr as usize) & mask;
        let i1r = (i0r + 1) & mask;
        let fl = pl - (pl as usize as f32);
        let fr = pr - (pr as usize as f32);
        (
            self.ring_l[i0l] * (1.0 - fl) + self.ring_l[i1l] * fl,
            self.ring_r[i0r] * (1.0 - fr) + self.ring_r[i1r] * fr,
        )
    }

    fn clear(&mut self) {
        self.ring_l.fill(0.0);
        self.ring_r.fill(0.0);
        self.write = 0;
    }
}

/// Flanger estereofónico: retardo muy corto (0-5 ms) modulado por LFO sinusoidal
/// con retroalimentación controlable. El "sweep" clásico de guitarras y sintetizadores.
#[derive(Debug, Clone)]
pub struct FlangerProcessor {
    ring: StereoDelayRing,
    phase: f32,
    sample_rate: f32,
    freq_hz: f32,
    depth: f32,
    feedback: f32,
    active: bool,
}

impl FlangerProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ring: StereoDelayRing::new(1024),
            phase: 0.0,
            sample_rate,
            freq_hz: default_flanger_freq(),
            depth: default_flanger_depth(),
            feedback: default_flanger_feedback(),
            active: false,
        }
    }

    pub fn set_params(&mut self, freq_hz: f32, depth: f32, feedback: f32, active: bool) {
        self.freq_hz = freq_hz.clamp(0.02, 10.0);
        self.depth = depth.clamp(0.0, 1.0);
        self.feedback = feedback.clamp(-0.95, 0.95);
        self.active = active;
        if !active {
            self.reset();
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.phase = 0.0;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active {
            return;
        }

        let phase_inc = (2.0 * PI * self.freq_hz) / self.sample_rate;
        // Retardo base 0.8 ms, barrido hasta 5 ms con el LFO
        let base_delay = 0.0008 * self.sample_rate;
        let sweep_delay = 0.005 * self.sample_rate * self.depth;
        let len = left.len().min(right.len());
        let fb = self.feedback;

        for i in 0..len {
            // Lectura del anillo ANTES de escribir (evita acople directo)
            let delay = base_delay + 0.5 * (1.0 + self.phase.sin()) * sweep_delay;
            let (dl, dr) = self.ring.read(delay, 1024);

            // Retroalimentación: la salida húmeda vuelve al anillo
            let mix = dl * 0.5 + dl * fb * 0.5;
            let mixr = dr * 0.5 + dr * fb * 0.5;

            self.ring.ring_l[self.ring.write] = left[i] + mix;
            self.ring.ring_r[self.ring.write] = right[i] + mixr;
            self.ring.write = (self.ring.write + 1) & 1023;

            left[i] += dl * 0.5;
            right[i] += dr * 0.5;

            self.phase += phase_inc;
            if self.phase >= 2.0 * PI {
                self.phase -= 2.0 * PI;
            }
        }
    }
}

/// Chorus estereofónico: retardo medio (5-20 ms) con LFO por canal desfasado 90°.
/// Produce el ensanchamiento orgánico de "multi-instrumentista" (Juno-106 / DX7).
#[derive(Debug, Clone)]
pub struct ChorusProcessor {
    ring: StereoDelayRing,
    phase_l: f32,
    phase_r: f32,
    sample_rate: f32,
    freq_hz: f32,
    depth: f32,
    mix: f32,
    active: bool,
}

impl ChorusProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ring: StereoDelayRing::new(4096),
            phase_l: 0.0,
            phase_r: PI / 2.0, // desfase de 90° entre canales
            sample_rate,
            freq_hz: default_chorus_freq(),
            depth: default_chorus_depth(),
            mix: default_chorus_mix(),
            active: false,
        }
    }

    pub fn set_params(&mut self, freq_hz: f32, depth: f32, mix: f32, active: bool) {
        self.freq_hz = freq_hz.clamp(0.05, 8.0);
        self.depth = depth.clamp(0.0, 1.0);
        self.mix = mix.clamp(0.0, 1.0);
        self.active = active;
        if !active {
            self.reset();
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.phase_l = 0.0;
        self.phase_r = PI / 2.0;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active {
            return;
        }

        let phase_inc = (2.0 * PI * self.freq_hz) / self.sample_rate;
        let base_delay = 0.008 * self.sample_rate;   // 8 ms
        let sweep = 0.012 * self.sample_rate * self.depth; // hasta 12 ms extra
        let len = left.len().min(right.len());
        let m = self.mix;

        for i in 0..len {
            // LFOs independientes por canal (desfase 90°)
            let delay_l = base_delay + 0.5 * (1.0 + self.phase_l.sin()) * sweep;
            let delay_r = base_delay + 0.5 * (1.0 + self.phase_r.sin()) * sweep;

            let (dl, dr) = self.ring.read(delay_l, 4096);

            self.ring.ring_l[self.ring.write] = left[i];
            self.ring.ring_r[self.ring.write] = right[i];
            self.ring.write = (self.ring.write + 1) & 4095;

            let (dl2, dr2) = self.ring.read(delay_r, 4096);

            left[i] += (dl + dl2) * 0.5 * m;
            right[i] += (dr + dr2) * 0.5 * m;

            self.phase_l += phase_inc;
            self.phase_r += phase_inc;
            if self.phase_l >= 2.0 * PI {
                self.phase_l -= 2.0 * PI;
            }
            if self.phase_r >= 2.0 * PI {
                self.phase_r -= 2.0 * PI;
            }
        }
    }
}

/// Phaser: 4 biquads paso-todos en cascada, frecuencia central modulada por LFO.
/// El barrido 360° de notches produce el "swirl" clásico de Moog / 12-string.
#[derive(Debug, Clone, Copy)]
struct AllPassFilter {
    g: f32,
    x1: f32,
    y1: f32,
}

impl AllPassFilter {
    /// Coeficiente g para frecuencia central a `freq_hz` (rango 0.0-1.0 de Nyquist)
    fn set_freq(&mut self, norm_freq: f32) {
        // g = (1 - 2πf) / (1 + 2πf)  →  f en Hz normalizado
        let f = norm_freq.clamp(0.001, 0.999);
        self.g = (1.0 - 2.0 * PI * f) / (1.0 + 2.0 * PI * f);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.x1 - self.g * self.y1 + self.g * x;
        self.x1 = x;
        self.y1 = y;
        y
    }

    fn clear(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

#[derive(Debug, Clone)]
pub struct PhaserProcessor {
    filters: [AllPassFilter; 8], // 4 por canal
    phase: f32,
    sample_rate: f32,
    freq_hz: f32,
    depth: f32,
    mix: f32,
    active: bool,
}

impl PhaserProcessor {
    pub fn new(sample_rate: f32) -> Self {
        let mut filters = [AllPassFilter { g: 0.5, x1: 0.0, y1: 0.0 }; 8];
        for f in filters.iter_mut() {
            f.set_freq(0.1);
        }
        Self {
            filters,
            phase: 0.0,
            sample_rate,
            freq_hz: default_phaser_freq(),
            depth: default_phaser_depth(),
            mix: default_phaser_mix(),
            active: false,
        }
    }

    pub fn set_params(&mut self, freq_hz: f32, depth: f32, mix: f32, active: bool) {
        self.freq_hz = freq_hz.clamp(0.01, 8.0);
        self.depth = depth.clamp(0.0, 1.0);
        self.mix = mix.clamp(0.0, 1.0);
        self.active = active;
        if !active {
            self.reset();
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn reset(&mut self) {
        for f in self.filters.iter_mut() {
            f.clear();
        }
        self.phase = 0.0;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active {
            return;
        }

        let phase_inc = (2.0 * PI * self.freq_hz) / self.sample_rate;
        let len = left.len().min(right.len());
        let m = self.mix;

        for i in 0..len {
            // La frecuencia central de los notches barre según el LFO (por muestra, barrido suave)
            let lfo = 0.5 * (1.0 + self.phase.sin());
            let center_norm = 0.005 + 0.06 * self.depth * lfo; // fracción de Nyquist: ~240 Hz a ~3 kHz a 48 kHz
            for f in self.filters.iter_mut() {
                f.set_freq(center_norm);
            }

            let mut l = left[i];
            let mut r = right[i];
            for f in self.filters.iter_mut().take(4) {
                l = f.process(l);
            }
            for f in self.filters.iter_mut().skip(4) {
                r = f.process(r);
            }
            left[i] = l * m + left[i] * (1.0 - m);
            right[i] = r * m + right[i] * (1.0 - m);

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
    pub flanger: FlangerProcessor,
    pub chorus: ChorusProcessor,
    pub phaser: PhaserProcessor,
    pub settings: ModulationSettings,
}

impl ModulationProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            rotation: RotationProcessor::new(sample_rate),
            tremolo: TremoloProcessor::new(sample_rate),
            vibrato: VibratoProcessor::new(sample_rate),
            flanger: FlangerProcessor::new(sample_rate),
            chorus: ChorusProcessor::new(sample_rate),
            phaser: PhaserProcessor::new(sample_rate),
            settings: ModulationSettings::default(),
        }
    }

    pub fn set_settings(&mut self, settings: ModulationSettings) {
        self.rotation.set_speed(settings.rotation_speed_hz);
        self.rotation.set_active(settings.rotation);

        self.tremolo.set_params(settings.tremolo_freq_hz, settings.tremolo_depth, settings.tremolo);
        self.vibrato.set_params(settings.vibrato_freq_hz, settings.vibrato_depth, settings.vibrato);
        self.flanger.set_params(
            settings.flanger_freq_hz,
            settings.flanger_depth,
            settings.flanger_feedback,
            settings.flanger,
        );
        self.chorus.set_params(
            settings.chorus_freq_hz,
            settings.chorus_depth,
            settings.chorus_mix,
            settings.chorus,
        );
        self.phaser.set_params(
            settings.phaser_freq_hz,
            settings.phaser_depth,
            settings.phaser_mix,
            settings.phaser,
        );

        self.settings = settings;
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.rotation.update_sample_rate(sample_rate);
        self.tremolo.update_sample_rate(sample_rate);
        self.vibrato.update_sample_rate(sample_rate);
        self.flanger.update_sample_rate(sample_rate);
        self.chorus.update_sample_rate(sample_rate);
        self.phaser.update_sample_rate(sample_rate);
    }

    pub fn reset(&mut self) {
        self.rotation.reset();
        self.tremolo.reset();
        self.vibrato.reset();
        self.flanger.reset();
        self.chorus.reset();
        self.phaser.reset();
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

        // 3. Flanger (retardo corto con feedback, sweep "alas de avión")
        self.flanger.process_planar(left, right);

        // 4. Chorus (retardo medio con LFO desfasado, ensanchamiento orgánico)
        self.chorus.process_planar(left, right);

        // 5. Phaser (barrido 360° de notches, "swirl" Moog)
        self.phaser.process_planar(left, right);

        // 6. Rotación 8D (paneo orbital en espacio acústico)
        self.rotation.process_planar(left, right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flanger_inactive_bypass() {
        let mut proc = FlangerProcessor::new(48000.0);
        assert!(!proc.is_active());
        let mut l = [0.5, -0.5];
        let mut r = [0.25, -0.25];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5, -0.5]);
        assert_eq!(r, [0.25, -0.25]);
    }

    #[test]
    fn test_flanger_processes_signal() {
        let mut proc = FlangerProcessor::new(48000.0);
        proc.set_params(0.4, 0.6, 0.4, true);
        assert!(proc.is_active());
        // Seno de 440 Hz de 0.5 s
        let n = 24000;
        let mut l = (0..n).map(|i| (i as f32 * 2.0 * PI * 440.0 / 48000.0).sin()).collect::<Vec<f32>>();
        let mut r = l.clone();
        let l0 = l[1000..1100].to_vec();
        proc.process_planar(&mut l, &mut r);
        // La señal debe seguir teniendo energía (no colapsada ni en silencio)
        let energy: f32 = l[1000..1100].iter().map(|s| s * s).sum();
        assert!(energy > 1.0, "El flanger debe conservar energía del seno");
        // Y debe diferir de la señal original (feedback + delay la modifican)
        let mut diff = 0.0f32;
        for i in 0..100 {
            diff += (l[1000 + i] - l0[i]).abs();
        }
        assert!(diff > 0.01, "El flanger debe modificar la señal");
    }

    #[test]
    fn test_chorus_inactive_bypass() {
        let mut proc = ChorusProcessor::new(48000.0);
        assert!(!proc.is_active());
        let mut l = [0.5];
        let mut r = [-0.25];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5]);
        assert_eq!(r, [-0.25]);
    }

    #[test]
    fn test_chorus_widens_stereo() {
        let mut proc = ChorusProcessor::new(48000.0);
        proc.set_params(0.8, 0.5, 0.5, true);
        let n = 8000;
        let mut l = (0..n).map(|i| (i as f32 * 2.0 * PI * 300.0 / 48000.0).sin()).collect::<Vec<f32>>();
        let mut r = l.clone();
        proc.process_planar(&mut l, &mut r);
        // Chorus añade húmedo: la RMS debe crecer respecto a la original
        let rms_in = (0..n).map(|i| ((i as f32 * 2.0 * PI * 300.0 / 48000.0).sin().powi(2)) as f64).sum::<f64>() / n as f64;
        let rms_out = l.iter().map(|s| *s as f64 * *s as f64).sum::<f64>() / n as f64;
        assert!(rms_out > rms_in, "El chorus debe sumar energía húmeda (rms_in={rms_in}, rms_out={rms_out})");
    }

    #[test]
    fn test_phaser_inactive_bypass() {
        let mut proc = PhaserProcessor::new(48000.0);
        assert!(!proc.is_active());
        let mut l = [0.5];
        let mut r = [-0.25];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5]);
        assert_eq!(r, [-0.25]);
    }

    #[test]
    fn test_phaser_notches_are_real() {
        // Con depth=0 el barrido es mínimo pero el filtro sigue activo: el output difiere del input
        let mut proc = PhaserProcessor::new(48000.0);
        proc.set_params(0.2, 0.0, 0.5, true);
        let n = 8000;
        let mut l = (0..n).map(|i| (i as f32 * 2.0 * PI * 1000.0 / 48000.0).sin()).collect::<Vec<f32>>();
        let mut r = vec![0.0f32; n];
        proc.process_planar(&mut l, &mut r);
        let mut diff = 0.0f32;
        for i in 100..n {
            let orig = (i as f32 * 2.0 * PI * 1000.0 / 48000.0).sin();
            diff += (l[i] - orig).abs();
        }
        assert!(diff > 1.0, "El phaser debe notchar la señal (diff={diff})");
    }

    #[test]
    fn test_modulation_all_off_is_bypass() {
        let mut proc = ModulationProcessor::new(48000.0);
        proc.set_settings(ModulationSettings::default());
        assert!(!proc.is_active());
        let mut l = [0.5, -0.25];
        let mut r = [0.125, 0.75];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5, -0.25]);
        assert_eq!(r, [0.125, 0.75]);
    }

    #[test]
    fn test_modulation_settings_defaults_inactive() {
        let s = ModulationSettings::default();
        assert!(!s.is_active());
        assert!(!s.flanger && !s.chorus && !s.phaser && !s.rotation && !s.tremolo && !s.vibrato);
    }
}
