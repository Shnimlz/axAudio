//! Decodificador envoltorio DSP para Songbird y Symphonia
//!
//! Intercepta las muestras PCM de 32-bit float antes del mixer de Songbird,
//! aplicando ecualización de 15 bandas, audio 3D estéreo y soft-limiter.
//! Garantiza procesamiento determinista y continuo sin pérdidas de tramas por contención.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal, SignalSpec};
use symphonia::core::codecs::{
    CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult,
};
use symphonia::core::errors::{Error as SymphError, Result as SymphResult};
use symphonia::core::formats::Packet;

use super::DspChain;

pub struct DspDecoder {
    inner: Box<dyn Decoder>,
    dsp: Arc<Mutex<DspChain>>,
    f32_buf: Option<AudioBuffer<f32>>,
    last_was_dsp: bool,
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

pub fn inject_dsp(parsed: &mut songbird::input::Parsed, dsp: Arc<Mutex<DspChain>>) {
    let dummy = Box::new(DummyDecoder);
    let original = std::mem::replace(&mut parsed.decoder, dummy);
    parsed.decoder = Box::new(DspDecoder::new(original, dsp));
}

impl DspDecoder {
    pub fn new(inner: Box<dyn Decoder>, dsp: Arc<Mutex<DspChain>>) -> Self {
        Self {
            inner,
            dsp,
            f32_buf: None,
            last_was_dsp: false,
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
        let mut dsp = self.dsp.lock().unwrap_or_else(|e| e.into_inner());
        dsp.spatial.reset();
    }

    fn codec_params(&self) -> &CodecParameters {
        self.inner.codec_params()
    }

    fn finalize(&mut self) -> FinalizeResult {
        self.inner.finalize()
    }

    fn decode(&mut self, packet: &Packet) -> SymphResult<AudioBufferRef<'_>> {
        // 1. Decodificar la trama con el códec subyacente (sin retener locks innecesarios)
        let decoded = self.inner.decode(packet)?;

        // 2. Adquirir lock determinista del DSP (operaciones en microsegundos, sin saltos por try_lock)
        let mut dsp = self.dsp.lock().unwrap_or_else(|e| e.into_inner());

        // 3. Si el DSP está completamente desactivado (modo flat/neutro), retornar directo
        // CERO sobrecarga o alteración de señal en modo neutro
        if !dsp.is_active() {
            self.last_was_dsp = false;
            return Ok(decoded);
        }

        self.last_was_dsp = true;

        // 4. Asegurar que f32_buf tenga EXACTAMENTE la misma capacidad y especificación que decoded
        // (CRUCIAL: Symphonia convert() usa dest.buf[c * self.n_capacity], por lo que
        // dest.n_capacity DEBE ser idéntico a self.n_capacity para no corromper el canal derecho)
        let need_realloc = match self.f32_buf {
            Some(ref buf) => buf.capacity() != decoded.capacity() || buf.spec() != decoded.spec(),
            None => true,
        };

        if need_realloc {
            self.f32_buf = Some(decoded.make_equivalent::<f32>());
        }

        let f32_buf = self.f32_buf.as_mut().unwrap();
        f32_buf.clear();
        decoded.convert(f32_buf);

        // 5. Aplicar la cadena DSP sobre los canales de audio plano
        if f32_buf.spec().channels.count() >= 2 {
            let (left, right) = f32_buf.chan_pair_mut(0, 1);
            dsp.process_planar(left, right);
        } else if f32_buf.spec().channels.count() == 1 {
            let mono = f32_buf.chan_mut(0);
            dsp.equalizer.process_mono(mono);
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
