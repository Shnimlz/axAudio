use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::broadcast;

use crate::dsp::{inject_dsp, DspParams};
use crate::models::PlayerEvent;
use crate::sources::TrackMetadata;

/// Estado de la máquina de estados del demuxer ICY
#[derive(Debug, PartialEq, Eq)]
enum IcyState {
    /// Leyendo bytes de audio del stream
    ReadingAudio,
    /// Leyendo el byte único de longitud de metadatos (L)
    ReadingLength,
    /// Leyendo los L * 16 bytes de metadatos
    ReadingMeta {
        remaining: usize,
        buffer: Vec<u8>,
    },
}

/// Envoltorio AsyncRead que demuxea metadatos ICY/Shoutcast de streams de radio en vivo.
///
/// Remueve de manera transparente los bloques de metadatos intercalados cada `metaint` bytes
/// de audio para que los decodificadores (Symphonia / Opus / AAC / MP3) reciban únicamente
/// bytes puros de audio sin errores de sincronización de cuadros, al tiempo que extrae
/// `StreamTitle` y emite eventos en tiempo real vía WebSocket.
pub struct IcyDemuxerStream<R> {
    inner: R,
    metaint: usize,
    bytes_until_meta: usize,
    state: IcyState,
    event_tx: Option<broadcast::Sender<PlayerEvent>>,
    guild_id: u64,
    bot_id: Option<u64>,
    last_title: Option<String>,
}

impl<R> IcyDemuxerStream<R> {
    pub fn new(
        inner: R,
        metaint: usize,
        event_tx: Option<broadcast::Sender<PlayerEvent>>,
        guild_id: u64,
        bot_id: Option<u64>,
    ) -> Self {
        Self {
            inner,
            metaint,
            bytes_until_meta: metaint,
            state: IcyState::ReadingAudio,
            event_tx,
            guild_id,
            bot_id,
            last_title: None,
        }
    }

    fn parse_and_emit_metadata(&mut self, raw: &str) {
        if let Some(stream_title) = parse_icy_title(raw) {
            let cleaned = stream_title.trim().trim_matches('\0').trim().to_string();
            if !cleaned.is_empty() && self.last_title.as_deref() != Some(&cleaned) {
                self.last_title = Some(cleaned.clone());
                let (artist, title) = split_artist_title(&cleaned);
                tracing::info!(
                    guild_id = self.guild_id,
                    bot_id = ?self.bot_id,
                    title = ?title,
                    artist = ?artist,
                    "Metadatos ICY de radio en vivo actualizados"
                );
                if let Some(ref tx) = self.event_tx {
                    let _ = tx.send(PlayerEvent::MetadataUpdate {
                        guild_id: self.guild_id,
                        bot_id: self.bot_id,
                        title,
                        artist,
                        raw: raw.to_string(),
                    });
                }
            }
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for IcyDemuxerStream<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        while buf.remaining() > 0 {
            match this.state {
                IcyState::ReadingAudio => {
                    if this.bytes_until_meta == 0 {
                        this.state = IcyState::ReadingLength;
                        continue;
                    }

                    let max_read = buf.remaining().min(this.bytes_until_meta);
                    let mut sub_buf = ReadBuf::new(buf.initialize_unfilled_to(max_read));
                    match Pin::new(&mut this.inner).poll_read(cx, &mut sub_buf) {
                        Poll::Ready(Ok(())) => {
                            let n = sub_buf.filled().len();
                            if n == 0 {
                                // EOF del stream subyacente
                                return Poll::Ready(Ok(()));
                            }
                            buf.advance(n);
                            this.bytes_until_meta -= n;
                            if this.bytes_until_meta == 0 {
                                this.state = IcyState::ReadingLength;
                            }
                            return Poll::Ready(Ok(()));
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                IcyState::ReadingLength => {
                    let mut byte = [0u8; 1];
                    let mut byte_buf = ReadBuf::new(&mut byte);
                    match Pin::new(&mut this.inner).poll_read(cx, &mut byte_buf) {
                        Poll::Ready(Ok(())) => {
                            if byte_buf.filled().is_empty() {
                                return Poll::Ready(Ok(())); // EOF
                            }
                            let len_byte = byte[0];
                            let meta_len = (len_byte as usize) * 16;
                            if meta_len == 0 {
                                this.bytes_until_meta = this.metaint;
                                this.state = IcyState::ReadingAudio;
                            } else {
                                this.state = IcyState::ReadingMeta {
                                    remaining: meta_len,
                                    buffer: Vec::with_capacity(meta_len),
                                };
                            }
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                IcyState::ReadingMeta { .. } => {
                    let to_read = match this.state {
                        IcyState::ReadingMeta { remaining, .. } => remaining.min(256),
                        _ => 0,
                    };
                    if to_read == 0 {
                        this.bytes_until_meta = this.metaint;
                        this.state = IcyState::ReadingAudio;
                        continue;
                    }

                    let mut chunk = [0u8; 256];
                    let mut chunk_buf = ReadBuf::new(&mut chunk[..to_read]);
                    match Pin::new(&mut this.inner).poll_read(cx, &mut chunk_buf) {
                        Poll::Ready(Ok(())) => {
                            let n = chunk_buf.filled().len();
                            if n == 0 {
                                return Poll::Ready(Ok(())); // EOF
                            }
                            let completed_meta = match &mut this.state {
                                IcyState::ReadingMeta { remaining, buffer } => {
                                    buffer.extend_from_slice(chunk_buf.filled());
                                    *remaining -= n;
                                    if *remaining == 0 {
                                        Some(String::from_utf8_lossy(buffer).to_string())
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                            if let Some(raw_meta) = completed_meta {
                                this.parse_and_emit_metadata(&raw_meta);
                                this.bytes_until_meta = this.metaint;
                                this.state = IcyState::ReadingAudio;
                            }
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// Extrae el campo `StreamTitle` de una cadena de metadatos ICY
pub fn parse_icy_title(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let marker = "streamtitle='";
    if let Some(idx) = lower.find(marker) {
        let after = &raw[idx + marker.len()..];
        if let Some(end_quote) = after.find("';") {
            return Some(after[..end_quote].to_string());
        } else if let Some(end_quote) = after.find('\'') {
            return Some(after[..end_quote].to_string());
        }
    }
    None
}

/// Separa una cadena de título en Artista y Título si contiene " - "
pub fn split_artist_title(s: &str) -> (Option<String>, Option<String>) {
    if let Some((artist, title)) = s.split_once(" - ") {
        (
            Some(artist.trim().to_string()),
            Some(title.trim().to_string()),
        )
    } else {
        (None, Some(s.trim().to_string()))
    }
}

/// Crea una pista Songbird conectada a una radio en vivo con demuxer ICY y pipeline DSP
pub async fn create_live_radio_track(
    http_client: reqwest::Client,
    stream_url: String,
    title: Option<String>,
    guild_id: u64,
    bot_id: Option<u64>,
    dsp_params: Arc<std::sync::RwLock<DspParams>>,
    event_tx: tokio::sync::broadcast::Sender<PlayerEvent>,
    custom_user_agent: Option<&str>,
) -> Result<(songbird::tracks::Track, Arc<TrackMetadata>), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::TryStreamExt;

    let req = http_client
        .get(&stream_url)
        .header("Icy-MetaData", "1")
        .header(
            "User-Agent",
            custom_user_agent.unwrap_or("axAudio-Radio/1.0 (Linux; Rust)"),
        );

    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(format!("El servidor de radio respondió con código HTTP {}", resp.status()).into());
    }

    let metaint: usize = resp
        .headers()
        .get("icy-metaint")
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<usize>().ok())
        .unwrap_or(0);

    let station_name = resp
        .headers()
        .get("icy-name")
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string());

    let final_title = title
        .or(station_name)
        .unwrap_or_else(|| "Live Radio Stream".to_string());

    let meta = Arc::new(TrackMetadata {
        title: final_title,
        source_url: stream_url.clone(),
        stream_url: stream_url.clone(),
        duration_secs: None,
        author: Some("Live Radio".to_string()),
        thumbnail: None,
        skip_segments: Vec::new(),
        stream_headers: Vec::new(),
        stream_content_length: None,
        resolved_at: std::time::Instant::now(),
        is_live: true,
        suggested_preset: None,
    });

    let byte_stream = resp.bytes_stream().map_err(std::io::Error::other);
    let async_reader = tokio_util::io::StreamReader::new(byte_stream);

    let input: songbird::input::Input = if metaint > 0 {
        tracing::info!(
            guild_id = guild_id,
            bot_id = ?bot_id,
            metaint = metaint,
            "Conectado a radio con protocolo ICY/Shoutcast metadata demuxer"
        );
        let demuxer = IcyDemuxerStream::new(async_reader, metaint, Some(event_tx), guild_id, bot_id);
        let src = songbird::input::AsyncReadOnlySource::new(demuxer);
        let adapter = songbird::input::AsyncAdapterStream::new(Box::new(src), 64 * 1024);
        let stream = songbird::input::AudioStream {
            input: Box::new(adapter) as Box<dyn symphonia::core::io::MediaSource>,
        };
        songbird::input::Input::Live(songbird::input::LiveInput::Raw(stream), None)
    } else {
        tracing::info!(
            guild_id = guild_id,
            bot_id = ?bot_id,
            "Conectado a radio streaming continuo (sin ICY intercalado)"
        );
        let src = songbird::input::AsyncReadOnlySource::new(async_reader);
        let adapter = songbird::input::AsyncAdapterStream::new(Box::new(src), 64 * 1024);
        let stream = songbird::input::AudioStream {
            input: Box::new(adapter) as Box<dyn symphonia::core::io::MediaSource>,
        };
        songbird::input::Input::Live(songbird::input::LiveInput::Raw(stream), None)
    };

    let mut playable = input
        .make_playable_async(
            songbird::input::codecs::get_codec_registry(),
            songbird::input::codecs::get_probe(),
        )
        .await
        .map_err(|e| format!("Error inicializando decodificador para stream de radio: {:?}", e))?;

    if let Some(parsed) = playable.parsed_mut() {
        inject_dsp(parsed, dsp_params);
    }

    Ok((songbird::tracks::Track::new_with_data(playable, meta.clone()), meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn test_parse_icy_title_standard() {
        let raw = "StreamTitle='Daft Punk - One More Time';StreamUrl='https://radio.example.com';";
        let parsed = parse_icy_title(raw);
        assert_eq!(parsed, Some("Daft Punk - One More Time".to_string()));

        let (artist, title) = split_artist_title(&parsed.unwrap());
        assert_eq!(artist, Some("Daft Punk".to_string()));
        assert_eq!(title, Some("One More Time".to_string()));
    }

    #[test]
    fn test_parse_icy_title_single_quotes() {
        let raw = "StreamTitle='Solo Station Identification';";
        let parsed = parse_icy_title(raw);
        assert_eq!(parsed, Some("Solo Station Identification".to_string()));

        let (artist, title) = split_artist_title(&parsed.unwrap());
        assert_eq!(artist, None);
        assert_eq!(title, Some("Solo Station Identification".to_string()));
    }

    #[tokio::test]
    async fn test_icy_demuxer_stream_stripping() {
        // Simular un stream: 10 bytes audio, 1 byte longitud (2), 32 bytes metadata, 10 bytes audio
        let metaint = 10;
        let mut simulated_stream: Vec<u8> = Vec::new();

        // 10 bytes audio A
        let audio_a = b"0123456789";
        simulated_stream.extend_from_slice(audio_a);

        // 1 byte longitud: 2 (2 * 16 = 32 bytes de metadatos)
        simulated_stream.push(2);

        // 32 bytes metadata
        let meta_str = "StreamTitle='Radio Test - Track';";
        let mut meta_bytes = meta_str.as_bytes().to_vec();
        meta_bytes.resize(32, 0);
        simulated_stream.extend_from_slice(&meta_bytes);

        // 10 bytes audio B
        let audio_b = b"ABCDEFGHIJ";
        simulated_stream.extend_from_slice(audio_b);

        let (tx, mut rx) = broadcast::channel(16);
        let cursor = std::io::Cursor::new(simulated_stream);
        let mut demuxer = IcyDemuxerStream::new(cursor, metaint, Some(tx), 12345, Some(6789));

        let mut output_audio = Vec::new();
        demuxer.read_to_end(&mut output_audio).await.unwrap();

        // El stream de salida debe contener ÚNICAMENTE los 20 bytes de audio combinados (A + B)
        assert_eq!(output_audio.len(), 20);
        assert_eq!(&output_audio[0..10], audio_a);
        assert_eq!(&output_audio[10..20], audio_b);

        // Verificar que se recibió el evento WebSocket de MetadataUpdate
        let event = rx.try_recv().expect("Debe emitirse un evento MetadataUpdate");
        match event {
            PlayerEvent::MetadataUpdate {
                guild_id,
                bot_id,
                title,
                artist,
                raw,
            } => {
                assert_eq!(guild_id, 12345);
                assert_eq!(bot_id, Some(6789));
                assert_eq!(title, Some("Track".to_string()));
                assert_eq!(artist, Some("Radio Test".to_string()));
                assert!(raw.contains("StreamTitle="));
            }
            _ => panic!("Tipo de evento inesperado"),
        }
    }
}
