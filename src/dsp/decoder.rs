//! Decodificador envoltorio DSP para Songbird y Symphonia
//!
//! Intercepta las muestras PCM de 32-bit float antes del mixer de Songbird,
//! aplicando ecualización de 15 bandas, audio 3D binaural y soft-limiter.

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
    f32_buf: AudioBuffer<f32>,
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
        let params = inner.codec_params();
        let sample_rate = params.sample_rate.unwrap_or(48000);
        let channels = params.channels.unwrap_or_else(|| symphonia::core::audio::Channels::FRONT_LEFT | symphonia::core::audio::Channels::FRONT_RIGHT);
        let spec = SignalSpec::new(sample_rate, channels);
        // Capacidad inicial para 100ms de audio (~4800 frames a 48kHz)
        let f32_buf = AudioBuffer::<f32>::new(4800, spec);

        Self {
            inner,
            dsp,
            f32_buf,
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
        if let Ok(mut dsp) = self.dsp.try_lock() {
            dsp.spatial.reset();
        }
    }

    fn codec_params(&self) -> &CodecParameters {
        self.inner.codec_params()
    }

    fn finalize(&mut self) -> FinalizeResult {
        self.inner.finalize()
    }

    fn decode(&mut self, packet: &Packet) -> SymphResult<AudioBufferRef<'_>> {
        let decoded = self.inner.decode(packet)?;

        // Asegurar que f32_buf tenga suficiente capacidad y el mismo SignalSpec
        let frames = decoded.frames();
        let spec = *decoded.spec();

        if self.f32_buf.capacity() < frames || self.f32_buf.spec() != &spec {
            self.f32_buf = AudioBuffer::<f32>::new((frames as u64).max(4800), spec);
        }

        self.f32_buf.clear();

        // Convertir la salida del decodificador a f32_buf
        match decoded {
            AudioBufferRef::F32(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            AudioBufferRef::S16(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            AudioBufferRef::S32(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            AudioBufferRef::U8(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            AudioBufferRef::S24(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            AudioBufferRef::F64(ref b) => {
                b.convert(&mut self.f32_buf);
            }
            _ => {
                // Si es un formato no cubierto, devolver sin procesar
                return self.inner.decode(packet);
            }
        }

        // Procesar las muestras estéreo a través de la cadena DSP
        if self.f32_buf.spec().channels.count() >= 2 {
            let (left, right) = self.f32_buf.chan_pair_mut(0, 1);
            if let Ok(mut dsp) = self.dsp.try_lock() {
                dsp.process_planar(left, right);
            }
        } else if self.f32_buf.spec().channels.count() == 1 {
            let mono = self.f32_buf.chan_mut(0);
            if let Ok(mut dsp) = self.dsp.try_lock() {
                dsp.equalizer.process_mono(mono);
                for sample in mono.iter_mut() {
                    *sample = (*sample * 0.95).tanh();
                }
            }
        }

        Ok(AudioBufferRef::F32(Cow::Borrowed(&self.f32_buf)))
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        AudioBufferRef::F32(Cow::Borrowed(&self.f32_buf))
    }
}
