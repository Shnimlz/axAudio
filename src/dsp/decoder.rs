//! Decodificador envoltorio DSP para Songbird y Symphonia
//!
//! Intercepta las muestras PCM de 32-bit float antes del mixer de Songbird,
//! aplicando ecualización de 15 bandas, audio 3D estéreo y soft-limiter.
//! Garantiza procesamiento determinista y continuo sin pérdidas de tramas por contención.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal, SignalSpec};
use symphonia::core::codecs::{
    CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult,
};
use symphonia::core::errors::{Error as SymphError, Result as SymphResult};
use symphonia::core::formats::Packet;

use super::{DspChain, DspParams};

pub struct DspDecoder {
    inner: Box<dyn Decoder>,
    params: Arc<RwLock<DspParams>>,
    seen_version: u64,
    chain: DspChain,
    f32_buf: Option<AudioBuffer<f32>>,
    last_was_dsp: bool,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
}

pub struct DummyDecoder;

impl Decoder for DummyDecoder {
    fn try_new(_: &CodecParameters, _: &DecoderOptions) -> SymphResult<Self> {
        Err(SymphError::Unsupported("placeholder"))
    }
    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[]
    }
    fn reset(&mut self) {}
    fn codec_params(&self) -> &CodecParameters {
        static PARAMS: std::sync::OnceLock<CodecParameters> = std::sync::OnceLock::new();
        PARAMS.get_or_init(CodecParameters::default)
    }
    fn decode(&mut self, _: &Packet) -> SymphResult<AudioBufferRef<'_>> {
        Err(SymphError::Unsupported("placeholder"))
    }
    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }
    fn last_decoded(&self) -> AudioBufferRef<'_> {
        static BUF: std::sync::OnceLock<AudioBuffer<f32>> = std::sync::OnceLock::new();
        let b = BUF.get_or_init(|| {
            AudioBuffer::new(
                0,
                SignalSpec::new(
                    48000,
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT,
                ),
            )
        });
        AudioBufferRef::F32(Cow::Borrowed(b))
    }
}

pub fn inject_dsp(parsed: &mut songbird::input::Parsed, params: Arc<RwLock<DspParams>>) {
    let dummy = Box::new(DummyDecoder);
    let original = std::mem::replace(&mut parsed.decoder, dummy);
    parsed.decoder = Box::new(DspDecoder::new(original, params));
}

impl DspDecoder {
    pub fn new(inner: Box<dyn Decoder>, params: Arc<RwLock<DspParams>>) -> Self {
        let (initial_params, initial_version) = match params.read() {
            Ok(p) => (p.clone(), p.version),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                (guard.clone(), guard.version)
            }
        };

        let mut chain = DspChain::new(48000.0);
        chain.rebuild(&initial_params, 48000.0);

        Self {
            inner,
            params,
            seen_version: initial_version,
            chain,
            f32_buf: None,
            last_was_dsp: false,
            out_l: Vec::with_capacity(4096),
            out_r: Vec::with_capacity(4096),
        }
    }
}

impl Decoder for DspDecoder {
    fn try_new(_params: &CodecParameters, _options: &DecoderOptions) -> SymphResult<Self>
    where
        Self: Sized,
    {
        Err(SymphError::Unsupported("DspDecoder debe envolver un Decoder existente"))
    }

    fn supported_codecs() -> &'static [CodecDescriptor]
    where
        Self: Sized,
    {
        &[]
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.last_was_dsp = false;
        self.chain.reset();
        self.out_l.clear();
        self.out_r.clear();
    }

    fn codec_params(&self) -> &CodecParameters {
        self.inner.codec_params()
    }

    fn finalize(&mut self) -> FinalizeResult {
        self.inner.finalize()
    }

    fn decode(&mut self, packet: &Packet) -> SymphResult<AudioBufferRef<'_>> {
        // 1. Decodificar la trama con el códec subyacente
        let decoded = self.inner.decode(packet)?;
        let rate = decoded.spec().rate as f32;

        // 2. Comprobación lock-free (try_read nunca bloquea el hilo de audio en tiempo real)
        // y adaptación automática a la tasa de muestreo nativa del stream
        if let Ok(p) = self.params.try_read() {
            if p.version != self.seen_version || (rate - self.chain.sample_rate).abs() > 1.0 {
                self.chain.rebuild(&p, rate);
                self.seen_version = p.version;
            }
        }

        // 3. Si el DSP está completamente desactivado (modo plano/neutro), retornar directo
        if !self.chain.is_active() {
            self.last_was_dsp = false;
            return Ok(decoded);
        }

        self.last_was_dsp = true;

        // 4. Asegurar que f32_buf tenga la capacidad y especificación EXACTA del paquete decodificado.
        // NOTA CRÍTICA: En Symphonia 0.5.5, `AudioBuffer::convert` indexa internamente con `self.n_capacity`
        // en lugar de `dest.n_capacity`. Por tanto, si f32_buf.capacity() != decoded.capacity(),
        // el canal 1 (derecho) se escribe en el offset equivocado y se lee silencio absoluto (0.0).
        let cap = decoded.capacity();
        let need_realloc = match self.f32_buf {
            Some(ref buf) => buf.capacity() != cap || buf.spec() != decoded.spec(),
            None => true,
        };

        if need_realloc {
            self.f32_buf = Some(AudioBuffer::new(cap as u64, *decoded.spec()));
        }

        let f32_buf = self.f32_buf.as_mut().unwrap();
        f32_buf.clear();
        decoded.convert(f32_buf);

        // 5. Aplicar la cadena DSP sobre los canales de audio plano
        if f32_buf.spec().channels.count() >= 2 {
            if self.chain.timescale.is_active() {
                self.out_l.clear();
                self.out_r.clear();
                {
                    let (left, right) = f32_buf.chan_pair_mut(0, 1);
                    self.chain.process_planar(left, right);
                    self.chain.timescale.process(left, right, &mut self.out_l, &mut self.out_r);
                }

                let out_len = self.out_l.len();
                if out_len > 0 {
                    if out_len > f32_buf.capacity() {
                        let new_cap = (out_len * 2).max(8192);
                        *f32_buf = AudioBuffer::new(new_cap as u64, *f32_buf.spec());
                    }
                    f32_buf.clear();
                    f32_buf.render_reserved(Some(out_len));
                    let (new_left, new_right) = f32_buf.chan_pair_mut(0, 1);
                    new_left.copy_from_slice(&self.out_l);
                    new_right.copy_from_slice(&self.out_r);
                }
            } else {
                let (left, right) = f32_buf.chan_pair_mut(0, 1);
                self.chain.process_planar(left, right);
            }
        } else if f32_buf.spec().channels.count() == 1 {
            let mono = f32_buf.chan_mut(0);
            self.chain.equalizer.process_mono(mono);
        }

        Ok(AudioBufferRef::F32(Cow::Borrowed(f32_buf)))
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        if self.last_was_dsp {
            if let Some(ref buf) = self.f32_buf {
                return AudioBufferRef::F32(Cow::Borrowed(buf));
            }
        }
        self.inner.last_decoded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::{Channels, Signal, SignalSpec};

    #[test]
    fn test_symphonia_convert_stereo_retention() {
        let spec = SignalSpec::new(48000, Channels::FRONT_LEFT | Channels::FRONT_RIGHT);
        let mut src = AudioBuffer::<f32>::new(960, spec);
        src.render_reserved(Some(960));
        let (left_src, right_src) = src.chan_pair_mut(0, 1);
        left_src.fill(1.0);
        right_src.fill(2.0);

        // When dest has exact matching capacity
        let mut dst = AudioBuffer::<f32>::new(src.capacity() as u64, spec);
        dst.clear();
        src.convert(&mut dst);

        let (left_dst, right_dst) = dst.chan_pair_mut(0, 1);
        assert_eq!(left_dst[0], 1.0, "Canal izquierdo debe conservar 1.0");
        assert_eq!(right_dst[0], 2.0, "Canal derecho debe conservar 2.0 y no ser silencio");
    }
}
