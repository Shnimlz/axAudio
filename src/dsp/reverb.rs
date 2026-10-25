//! Reverberación estereofónica por Red de Difusión con Feed-Forward (FDN, J. Chowning 1980)
//!
//! Ocho nodos de retardo en cascada serie (salida de cada nodo → entrada del siguiente)
//! con longitudes mutuamente coprimas (números primos) para maximizar la
//! pseudo-aleatoriedad de la cola. Pre-delay de "sala pequeña" (15 ms) separa el golpe
//! directo de la primera cola. La ruta seca NUNCA sufre filtrado en peine: la señal
//! directa no pasa por los nodos, la cola es la salida del último nodo.
//!
//! - RT60 configurable (0.2 a 6.0 s): ganancia por nodo g = 10^(-60/(20·RT60·fs)),
//!   así la cascada de 8 nodos decae exactamente a -60 dB en RT60 segundos
//! - Tono de cola regulable: FLP 1-polo por nodo barrido de 1.5 kHz (oscuro) a 11 kHz (brillante)
//! - HLP 1-polo de entrada (~120 Hz) evita acumulación de sub-graves en la cola
//! - Salida normalizada por la ganancia acumulada (g^8) → el wet es perceptual

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// Longitudes de las 8 líneas de retardo en ms (números primos → comb comb-free)
const FDN_DELAYS_MS: [u32; 8] = [25, 36, 41, 49, 55, 64, 77, 93];

/// Anillo de retardo: potencia de 2 mayor que el mayor retardo a 48 kHz (93 ms ≈ 4464 muestras)
const FDN_RING: usize = 8192;
const FDN_MASK: usize = FDN_RING - 1;

/// Pre-delay de sala pequeña (separa ataque directo de la primera cola)
const PRE_DELAY_MS: u32 = 15;
const PRED_RING: usize = 1024;
const PRED_MASK: usize = PRED_RING - 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverbSettings {
    #[serde(default)]
    pub reverb: bool,
    /// Tamaño de sala: RT60 en segundos (0.2 a 6.0, default 1.8)
    #[serde(default = "default_reverb_size")]
    pub reverb_room_size: f32,
    /// Tono de la cola (0.0 oscuro a 1.0 brillante, default 0.5)
    #[serde(default = "default_reverb_tone")]
    pub reverb_tone: f32,
    /// Nivel de mezcla húmeda (0.0 a 1.0, default 0.3)
    #[serde(default = "default_reverb_wet")]
    pub reverb_wet: f32,
}

fn default_reverb_size() -> f32 {
    1.8
}
fn default_reverb_tone() -> f32 {
    0.5
}
fn default_reverb_wet() -> f32 {
    0.3
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            reverb: false,
            reverb_room_size: default_reverb_size(),
            reverb_tone: default_reverb_tone(),
            reverb_wet: default_reverb_wet(),
        }
    }
}

/// Retardo fraccional por anillo circular + interpolación lineal (sin comb en ruta seca)
#[derive(Debug, Clone)]
struct CombDelay {
    ring: Vec<f32>,
    write: usize,
    length: f32,
}

impl CombDelay {
    fn new() -> Self {
        Self {
            ring: vec![0.0; FDN_RING],
            write: 0,
            length: 1.0,
        }
    }

    fn set_length(&mut self, length: f32) {
        self.length = length.clamp(4.0, FDN_RING as f32 * 0.5);
    }

    #[inline]
    fn write(&mut self, sample: f32) {
        self.ring[self.write] = sample;
        self.write = (self.write + 1) & FDN_MASK;
    }

    #[inline]
    fn read(&self) -> f32 {
        let mut pos = self.write as f32 - self.length;
        while pos < 0.0 {
            pos += FDN_RING as f32;
        }
        let i0 = (pos as usize) & FDN_MASK;
        let i1 = (i0 + 1) & FDN_MASK;
        let frac = pos - (pos as usize as f32);
        self.ring[i0] * (1.0 - frac) + self.ring[i1] * frac
    }

    fn clear(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
    }
}

/// Filtro 1-polo (baja complejidad, cero latencia adicional, fase suave)
#[derive(Debug, Clone)]
struct OnePole {
    alpha: f32,
    z1: f32,
}

impl OnePole {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let alpha = (2.0 * PI * (cutoff_hz / sample_rate)).tan();
        Self { alpha, z1: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.z1 += self.alpha * (x - self.z1);
        self.z1
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
    }
}

#[derive(Debug, Clone)]
pub struct ReverbProcessor {
    delays_l: [CombDelay; 8],
    delays_r: [CombDelay; 8],
    /// Pre-delay estéreo (15 ms)
    pred_ring_l: Vec<f32>,
    pred_ring_r: Vec<f32>,
    pred_write: usize,
    pred_length: f32,
    /// EQ de tono por línea (FLP: 1.5–11 kHz) — índice 0..8 izquierda, 8..16 derecha
    lfp: [OnePole; 16],
    /// HLP de entrada (~120 Hz): uno por canal
    hlp_in_l: OnePole,
    hlp_in_r: OnePole,
    /// Ganancia de resorte por nodo (del RT60 deseado)
    gain: f32,
    /// Ganancia acumulada de la cascada (g^8) para normalizar la salida húmeda
    gain_pow8: f32,
    /// Señal de cola de la iteración previa (retroalimentación del bucle FDN)
    loop_l: f32,
    loop_r: f32,
    wet: f32,
    active: bool,
    sample_rate: f32,
}

impl ReverbProcessor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            delays_l: std::array::from_fn(|_| CombDelay::new()),
            delays_r: std::array::from_fn(|_| CombDelay::new()),
            pred_ring_l: vec![0.0; PRED_RING],
            pred_ring_r: vec![0.0; PRED_RING],
            pred_write: 0,
            pred_length: (PRE_DELAY_MS as f32 / 1000.0 * sample_rate).clamp(2.0, PRED_RING as f32 * 0.5),
            lfp: std::array::from_fn(|_| OnePole::new(5000.0, sample_rate)),
            hlp_in_l: OnePole::new(120.0, sample_rate),
            hlp_in_r: OnePole::new(120.0, sample_rate),
            gain: 0.5,
            gain_pow8: 0.5f32.powi(8),
            loop_l: 0.0,
            loop_r: 0.0,
            wet: default_reverb_wet(),
            active: false,
            sample_rate,
        }
    }

    pub fn set_settings(&mut self, settings: ReverbSettings) {
        let rt60 = settings.reverb_room_size.clamp(0.2, 6.0);
        // FDN en cascada serie con bucle: la señal recorre los 8 nodos (retardo total
        // D) y vuelve al inicio atenuada g^8. La pérdida por recorrido es 20·log10(g^8)
        // dB cada D segundos → g^8 = 10^(-60/(20·RT60·(D/fs))).
        let total_delay_samples: f32 = FDN_DELAYS_MS.iter().sum::<u32>() as f32 / 1000.0 * self.sample_rate;
        let loops_per_rt60 = rt60 * self.sample_rate / total_delay_samples;
        let g8 = 10.0f32.powf(-60.0 / (20.0 * loops_per_rt60));
        self.gain = g8.powf(1.0 / 8.0).clamp(0.1, 0.98);
        self.gain_pow8 = self.gain.powi(8).max(1e-9);

        let tone = settings.reverb_tone.clamp(0.0, 1.0);
        let lfp_hz = 1500.0 + 9500.0 * tone;
        for n in 0..8 {
            let len = FDN_DELAYS_MS[n] as f32 / 1000.0 * self.sample_rate;
            self.delays_l[n].set_length(len);
            self.delays_r[n].set_length(len);
            self.lfp[n] = OnePole::new(lfp_hz, self.sample_rate);
            self.lfp[n + 8] = OnePole::new(lfp_hz, self.sample_rate);
        }

        self.wet = settings.reverb_wet.clamp(0.0, 1.0);
        self.active = settings.reverb && self.wet > 0.001;
        if !self.active {
            self.reset();
        }
    }

    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.pred_length = (PRE_DELAY_MS as f32 / 1000.0 * sample_rate).clamp(2.0, PRED_RING as f32 * 0.5);
        // Longitudes y FLP se re-derivan en el próximo set_settings (ya ocurre en rebuild)
    }

    pub fn reset(&mut self) {
        for d in self.delays_l.iter_mut() {
            d.clear();
        }
        for d in self.delays_r.iter_mut() {
            d.clear();
        }
        self.pred_ring_l.fill(0.0);
        self.pred_ring_r.fill(0.0);
        self.pred_write = 0;
        for f in self.lfp.iter_mut() {
            f.clear();
        }
        self.hlp_in_l.clear();
        self.hlp_in_r.clear();
        self.loop_l = 0.0;
        self.loop_r = 0.0;
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    fn pred_read(&self, ring: &[f32]) -> f32 {
        let mut pos = self.pred_write as f32 - self.pred_length;
        while pos < 0.0 {
            pos += PRED_RING as f32;
        }
        let i0 = (pos as usize) & PRED_MASK;
        let i1 = (i0 + 1) & PRED_MASK;
        let frac = pos - (pos as usize as f32);
        ring[i0] * (1.0 - frac) + ring[i1] * frac
    }

    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.active {
            return;
        }

        let len = left.len().min(right.len());
        let g = self.gain;
        let wet_gain = self.wet;

        for i in 0..len {
            // 1. Ingestar en el pre-delay estéreo
            self.pred_ring_l[self.pred_write] = left[i];
            self.pred_ring_r[self.pred_write] = right[i];
            self.pred_write = (self.pred_write + 1) & PRED_MASK;

            // 2. Extraer tras pre-delay y aplicar HLP de entrada (anti-sub en cola)
            let eq_l = self.hlp_in_l.process(self.pred_read(&self.pred_ring_l));
            let eq_r = self.hlp_in_r.process(self.pred_read(&self.pred_ring_r));

            // 3. FDN en cascada serie con bucle (Chowning): la señal de cola (previa
            //    iteración, salida del último nodo) retorna al inicio atenuada, y la
            //    nueva entrada se inyecta en paralelo. La señal seca NUNCA pasa por los
            //    nodos → la ruta directa no sufre comb.
            let loop_l = self.loop_l;
            let loop_r = self.loop_r;
            let mut sig_l = eq_l + loop_l;
            let mut sig_r = eq_r + loop_r;
            for n in 0..8 {
                self.delays_l[n].write(sig_l);
                self.delays_r[n].write(sig_r);
                sig_l = self.lfp[n].process(self.delays_l[n].read()) * g;
                sig_r = self.lfp[n + 8].process(self.delays_r[n].read()) * g;
            }

            // 4. Salida del último nodo: la cola reverberada (también retroalimentada)
            self.loop_l = sig_l;
            self.loop_r = sig_r;
            let out_l = sig_l;
            let out_r = sig_r;

            // 5. Mezcla húmeda aditiva
            left[i] += out_l * wet_gain;
            right[i] += out_r * wet_gain;
        }
    }
}

impl Default for ReverbProcessor {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverb_inactive_bypass() {
        let mut proc = ReverbProcessor::new(48000.0);
        assert!(!proc.is_active());
        let mut l = [0.5, -0.5];
        let mut r = [0.25, -0.25];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5, -0.5]);
        assert_eq!(r, [0.25, -0.25]);
    }

    #[test]
    fn test_reverb_adds_tail() {
        let mut proc = ReverbProcessor::new(48000.0);
        proc.set_settings(ReverbSettings {
            reverb: true,
            reverb_room_size: 1.8,
            reverb_tone: 0.5,
            reverb_wet: 0.5,
        });
        assert!(proc.is_active());
        // Señal de prueba: ruido pseudo-aleatorio de 1 segundo
        let n = 48000;
        let mut l = (0..n).map(|i| {
            let x = ((i as f64 * 12743.0).sin() as f32) * 0.5;
            x
        }).collect::<Vec<f32>>();
        let mut r = l.clone();
        let rms_in = (l.iter().map(|s| *s as f64 * *s as f64).sum::<f64>() / n as f64).sqrt() as f32;
        proc.process_planar(&mut l, &mut r);
        let rms_out = (l.iter().map(|s| *s as f64 * *s as f64).sum::<f64>() / n as f64).sqrt() as f32;
        // La reverb debe aumentar la RMS (añade energía húmeda)
        assert!(
            rms_out > rms_in,
            "La reverb debe aumentar la RMS de la señal (in={rms_in:.4}, out={rms_out:.4})"
        );
    }

    #[test]
    fn test_reverb_rt60_longer_than_short_room() {
        /// RMS de la "cola" (segundo 4-5, donde la señal original ya se detuvo)
        /// usando un burst de 2 segundos de ruido blanco
        fn tail_rms(rt60: f32) -> f32 {
            let mut proc = ReverbProcessor::new(48000.0);
            proc.set_settings(ReverbSettings {
                reverb: true,
                reverb_room_size: rt60,
                reverb_tone: 0.5,
                reverb_wet: 0.5,
            });
            let n = 48000 * 5; // 5 segundos
            // Burst de 2 segundos de ruido blanco, luego silencio
            let mut l = (0..n).map(|i| {
                if i < 48000 * 2 {
                    ((i as f64 * 12743.0).sin() as f32) * 0.5
                } else {
                    0.0
                }
            }).collect::<Vec<f32>>();
            let mut r = l.clone();
            proc.process_planar(&mut l, &mut r);
            // Medir la cola en el segundo 4-5 (después del burst de 2s)
            let mut sum = 0.0f64;
            let count = 48000;
            for s in l.iter().skip(48000 * 4).take(count) {
                sum += (*s as f64) * (*s as f64);
            }
            (sum / count as f64).sqrt() as f32
        }
        let big = tail_rms(4.0);
        let small = tail_rms(0.5);
        assert!(
            big > small,
            "RT60=4s debe tener más cola en t=4s que RT60=0.5s: big={big:.6}, small={small:.6}"
        );
    }

    #[test]
    fn test_reverb_zero_wet_bypass() {
        let mut proc = ReverbProcessor::new(48000.0);
        proc.set_settings(ReverbSettings {
            reverb: true,
            reverb_wet: 0.0,
            ..Default::default()
        });
        assert!(!proc.is_active());
        let mut l = [0.5];
        let mut r = [0.5];
        proc.process_planar(&mut l, &mut r);
        assert_eq!(l, [0.5]);
        assert_eq!(r, [0.5]);
    }
}
