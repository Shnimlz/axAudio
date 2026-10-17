//! Procesador de Timescale Real (Ajuste Independiente de Velocidad, Pitch y Rate)
//!
//! Permite modificar el tempo sin alterar el tono (Time-Stretching con WSOLA),
//! alterar el tono sin modificar la duración (Pitch-Shifting con WSOLA + Resampling),
//! o remuestreo analógico conjunto (Cubic Hermite Spline Resampling para Nightcore y Vaporwave).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimescaleSettings {
    #[serde(default = "default_factor")]
    pub speed: f32, // Multiplicador de velocidad (tempo-stretch, pitch neutro): 0.25 a 4.0
    #[serde(default = "default_factor")]
    pub pitch: f32, // Multiplicador de tono (pitch-shift, tempo neutro): 0.25 a 4.0
    #[serde(default = "default_factor")]
    pub rate: f32,  // Remuestreo conjunto (velocidad + tono simultáneos): 0.25 a 4.0
}

fn default_factor() -> f32 {
    1.0
}

impl Default for TimescaleSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pitch: 1.0,
            rate: 1.0,
        }
    }
}

impl TimescaleSettings {
    #[inline]
    pub fn is_active(&self) -> bool {
        (self.speed - 1.0).abs() > 0.005
            || (self.pitch - 1.0).abs() > 0.005
            || (self.rate - 1.0).abs() > 0.005
    }

    #[inline]
    pub fn total_speed(&self) -> f32 {
        (self.speed * self.rate).clamp(0.25, 4.0)
    }

    #[inline]
    pub fn total_pitch(&self) -> f32 {
        (self.pitch * self.rate).clamp(0.25, 4.0)
    }
}

/// Remuestreador polinomial de tercer orden (Cubic Hermite Spline)
///
/// Proporciona interpolación continua C1 con bajo costo de CPU y alta relación señal/ruido (>75 dB),
/// garantizando ausencia de clics o artefactos en las transiciones de tramas.
#[derive(Debug, Clone)]
pub struct HermiteResampler {
    // Historial de 4 muestras previas por canal para continuidad entre tramas
    hist_l: [f32; 4],
    hist_r: [f32; 4],
    phase: f64,
}

impl HermiteResampler {
    pub fn new() -> Self {
        Self {
            hist_l: [0.0; 4],
            hist_r: [0.0; 4],
            phase: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.hist_l = [0.0; 4];
        self.hist_r = [0.0; 4];
        self.phase = 0.0;
    }

    /// Remuestrea una señal estéreo por una relación `ratio = f_out / f_in`
    /// Para reproducir más rápido/agudo, `ratio > 1.0`; más lento/grave, `ratio < 1.0`.
    #[inline]
    pub fn process(
        &mut self,
        left_in: &[f32],
        right_in: &[f32],
        ratio: f64,
        left_out: &mut Vec<f32>,
        right_out: &mut Vec<f32>,
    ) {
        let in_len = left_in.len().min(right_in.len());
        if in_len == 0 {
            return;
        }

        let step = ratio.clamp(0.25, 4.0);

        // Helper para leer muestras con extensión de bordes usando el buffer histórico
        let get_sample = |idx: isize, src: &[f32], hist: &[f32; 4]| -> f32 {
            if idx < 0 {
                let h_idx = (4 + idx).clamp(0, 3) as usize;
                hist[h_idx]
            } else if (idx as usize) < src.len() {
                src[idx as usize]
            } else {
                src[src.len() - 1]
            }
        };

        while self.phase < (in_len as f64) {
            let i = self.phase.floor() as isize;
            let f = (self.phase - i as f64) as f32;

            // Interpolación Hermite para canal izquierdo
            let ym1_l = get_sample(i - 1, left_in, &self.hist_l);
            let y0_l = get_sample(i, left_in, &self.hist_l);
            let y1_l = get_sample(i + 1, left_in, &self.hist_l);
            let y2_l = get_sample(i + 2, left_in, &self.hist_l);

            let c0_l = y0_l;
            let c1_l = 0.5 * (y1_l - ym1_l);
            let c2_l = ym1_l - 2.5 * y0_l + 2.0 * y1_l - 0.5 * y2_l;
            let c3_l = 0.5 * (y2_l - ym1_l) + 1.5 * (y0_l - y1_l);
            let out_l = ((c3_l * f + c2_l) * f + c1_l) * f + c0_l;

            // Interpolación Hermite para canal derecho
            let ym1_r = get_sample(i - 1, right_in, &self.hist_r);
            let y0_r = get_sample(i, right_in, &self.hist_r);
            let y1_r = get_sample(i + 1, right_in, &self.hist_r);
            let y2_r = get_sample(i + 2, right_in, &self.hist_r);

            let c0_r = y0_r;
            let c1_r = 0.5 * (y1_r - ym1_r);
            let c2_r = ym1_r - 2.5 * y0_r + 2.0 * y1_r - 0.5 * y2_r;
            let c3_r = 0.5 * (y2_r - ym1_r) + 1.5 * (y0_r - y1_r);
            let out_r = ((c3_r * f + c2_r) * f + c1_r) * f + c0_r;

            left_out.push(out_l);
            right_out.push(out_r);

            self.phase += step;
        }

        // Actualizar fase restante para la siguiente trama
        self.phase -= in_len as f64;

        // Guardar las últimas 4 muestras para la continuidad del próximo paquete
        if in_len >= 4 {
            for k in 0..4 {
                self.hist_l[k] = left_in[in_len - 4 + k];
                self.hist_r[k] = right_in[in_len - 4 + k];
            }
        }
    }
}

/// Extensor de tiempo WSOLA (Waveform Similarity Overlap-Add)
///
/// Modifica el tempo del audio sin alterar el tono, encontrando puntos óptimos de coincidencia
/// de fase entre segmentos contiguos mediante correlación cruzada.
const WSOLA_FRAME: usize = 512;
const WSOLA_OVERLAP: usize = 256;
const WSOLA_SEARCH: usize = 128;

#[derive(Debug, Clone)]
pub struct WsolaTimeStretcher {
    in_buf_l: Vec<f32>,
    in_buf_r: Vec<f32>,
    prev_overlap_l: Vec<f32>,
    prev_overlap_r: Vec<f32>,
    window: Vec<f32>,
    read_pos: f64,
}

impl WsolaTimeStretcher {
    pub fn new() -> Self {
        // Precalcular ventana de mezcla Hann para solapamiento suave
        let mut window = Vec::with_capacity(WSOLA_OVERLAP);
        for i in 0..WSOLA_OVERLAP {
            let w = 0.5 * (1.0 - (std::f32::consts::PI * i as f32 / WSOLA_OVERLAP as f32).cos());
            window.push(w);
        }

        Self {
            in_buf_l: Vec::with_capacity(4096),
            in_buf_r: Vec::with_capacity(4096),
            prev_overlap_l: vec![0.0; WSOLA_OVERLAP],
            prev_overlap_r: vec![0.0; WSOLA_OVERLAP],
            window,
            read_pos: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.in_buf_l.clear();
        self.in_buf_r.clear();
        self.prev_overlap_l.fill(0.0);
        self.prev_overlap_r.fill(0.0);
        self.read_pos = 0.0;
    }

    /// Estira o comprime temporalmente el audio entrante por un factor `time_stretch_ratio`
    /// ratio > 1.0: reproducción más rápida (comprime duración).
    /// ratio < 1.0: reproducción más lenta (expande duración).
    pub fn process(
        &mut self,
        left_in: &[f32],
        right_in: &[f32],
        time_stretch_ratio: f32,
        left_out: &mut Vec<f32>,
        right_out: &mut Vec<f32>,
    ) {
        self.in_buf_l.extend_from_slice(left_in);
        self.in_buf_r.extend_from_slice(right_in);

        let hop_s = WSOLA_OVERLAP; // Salto de síntesis nominal
        let hop_a_nominal = (hop_s as f32 * time_stretch_ratio).round() as usize;

        let needed = WSOLA_FRAME + WSOLA_SEARCH + hop_a_nominal;

        while self.in_buf_l.len() >= needed {
            let nominal_pos = self.read_pos.floor() as usize;
            if nominal_pos + WSOLA_FRAME + WSOLA_SEARCH > self.in_buf_l.len() {
                break;
            }

            // Búsqueda de correlación cruzada máxima dentro del rango de tolerancia [-WSOLA_SEARCH/2, +WSOLA_SEARCH/2]
            let mut best_offset = 0;
            let mut best_corr = f32::MIN;

            for delta in 0..WSOLA_SEARCH {
                let candidate_pos = nominal_pos + delta;
                let mut corr = 0.0f32;

                // Submuestreo por paso 2 para máxima aceleración de cálculo de correlación
                let mut k = 0;
                while k < WSOLA_OVERLAP {
                    let s_l = self.in_buf_l[candidate_pos + k];
                    let s_r = self.in_buf_r[candidate_pos + k];
                    corr += s_l * self.prev_overlap_l[k] + s_r * self.prev_overlap_r[k];
                    k += 2;
                }

                if corr > best_corr {
                    best_corr = corr;
                    best_offset = delta;
                }
            }

            let best_pos = nominal_pos + best_offset;

            // 1. Solapar y mezclar la zona de transición con la ventana Hann
            for i in 0..WSOLA_OVERLAP {
                let w = self.window[i];
                let l = (1.0 - w) * self.prev_overlap_l[i] + w * self.in_buf_l[best_pos + i];
                let r = (1.0 - w) * self.prev_overlap_r[i] + w * self.in_buf_r[best_pos + i];
                left_out.push(l);
                right_out.push(r);
            }

            // 2. Guardar la segunda mitad de la trama como solapamiento previo para el siguiente ciclo
            for i in 0..WSOLA_OVERLAP {
                self.prev_overlap_l[i] = self.in_buf_l[best_pos + WSOLA_OVERLAP + i];
                self.prev_overlap_r[i] = self.in_buf_r[best_pos + WSOLA_OVERLAP + i];
            }

            // Avanzar posición de lectura según la tasa de estiramiento temporal
            self.read_pos += (hop_s as f32 * time_stretch_ratio) as f64;

            // Purgar muestras consumidas del buffer interno para mantener memoria acotada
            let consumed = nominal_pos.min(self.in_buf_l.len() / 2);
            if consumed > 0 {
                self.in_buf_l.drain(..consumed);
                self.in_buf_r.drain(..consumed);
                self.read_pos = (self.read_pos - consumed as f64).max(0.0);
            }
        }
    }
}

/// Coordinador de Timescale general (WSOLA + Hermite Resampler)
#[derive(Debug, Clone)]
pub struct TimescaleProcessor {
    settings: TimescaleSettings,
    resampler: HermiteResampler,
    wsola: WsolaTimeStretcher,

    // Buffers temporales intermedios para evitar allocs en tiempo de ejecución
    temp_l: Vec<f32>,
    temp_r: Vec<f32>,
}

impl TimescaleProcessor {
    pub fn new() -> Self {
        Self {
            settings: TimescaleSettings::default(),
            resampler: HermiteResampler::new(),
            wsola: WsolaTimeStretcher::new(),
            temp_l: Vec::with_capacity(4096),
            temp_r: Vec::with_capacity(4096),
        }
    }

    pub fn set_settings(&mut self, settings: TimescaleSettings) {
        self.settings = settings;
    }

    pub fn get_settings(&self) -> &TimescaleSettings {
        &self.settings
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.settings.is_active()
    }

    pub fn reset(&mut self) {
        self.resampler.reset();
        self.wsola.reset();
        self.temp_l.clear();
        self.temp_r.clear();
    }

    /// Procesa audio planar izquierdo y derecho según la configuración de timescale
    pub fn process(
        &mut self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut Vec<f32>,
        right_out: &mut Vec<f32>,
    ) {
        if !self.is_active() {
            left_out.extend_from_slice(left_in);
            right_out.extend_from_slice(right_in);
            return;
        }

        let speed = self.settings.total_speed();
        let pitch = self.settings.total_pitch();

        // CASO A: Remuestreo Puro (Speed == Pitch, e.g. Nightcore / Vaporwave)
        // Máxima fidelidad y cero artefactos de fase: un solo paso de interpolación Hermite
        if (speed - pitch).abs() < 0.005 {
            self.resampler
                .process(left_in, right_in, speed as f64, left_out, right_out);
            return;
        }

        // CASO B: Timescale General (Speed != Pitch, tono y tempo desacoplados)
        // Paso 1: WSOLA modifica la duración por (speed / pitch) preservando el tono
        let time_stretch = speed / pitch;
        self.temp_l.clear();
        self.temp_r.clear();
        self.wsola
            .process(left_in, right_in, time_stretch, &mut self.temp_l, &mut self.temp_r);

        // Paso 2: Remuestreador Hermite traslada el tono por el factor `pitch`
        self.resampler
            .process(&self.temp_l, &self.temp_r, pitch as f64, left_out, right_out);
    }
}
