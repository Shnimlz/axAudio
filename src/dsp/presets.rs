//! Presets de ecualización y procesamiento 3D calibrados para audiófilos
//!
//! Curvas de respuesta acústica de estudio, ampliación Mid/Side, sonido envolvente y normalización EBU R128.

use super::equalizer::Equalizer;
use super::loudness::LoudnessSettings;
use super::modulation::ModulationSettings;
use super::spatial::{SpatialProcessor, SpatialSettings};

pub struct DspPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub eq_bands: [f32; 15], // dB por banda (25Hz a 16kHz)
    pub spatial: SpatialSettings,
    pub modulation: ModulationSettings,
    pub loudness: LoudnessSettings,
}

pub fn get_preset(name: &str) -> Option<DspPreset> {
    match name.to_lowercase().replace('_', "-").as_str() {
        // --- Music / Hi-Fi ---
        "music" | "hifi" | "ax-music" | "fx-music" => Some(DspPreset {
            name: "music",
            description: "Calibración Hi-Fi: graves cálidos y profundos, medios transparentes y agudos sedosos sin fatiga auditiva",
            // 25, 40, 63, 100, 160, 250, 400, 630, 1k, 1.6k, 2.5k, 4k, 6.3k, 10k, 16k
            eq_bands: [2.5, 2.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0],
            spatial: SpatialSettings {
                stereo_width: 1.18,
                binaural_crossfeed: true,
                crossfeed_level: 0.12,
                bass_exciter: true,
                bass_exciter_drive: 0.15,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Cinema 3D / Dolby Atmos ---
        "cinema-3d" | "dolby-atmos" | "atmos" | "cinema" | "movie" | "fx-cinema" => Some(DspPreset {
            name: "cinema-3d",
            description: "Inmersión cinemática 3D tipo Dolby Atmos con ampliación Mid/Side, diálogos cristalinos y sub-graves profundos",
            eq_bands: [3.5, 3.0, 2.0, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0],
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.18,
                bass_exciter: true,
                bass_exciter_drive: 0.22,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Bass Boost 3D ---
        "bass-boost-3d" | "bass-boost" | "bass" | "ax-bass" | "fx-bass" => Some(DspPreset {
            name: "bass-boost-3d",
            description: "Realce visceral de sub-graves con armónicos limpios y escena ampliada sin distorsión áspera",
            eq_bands: [5.0, 4.5, 3.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.5, 2.0, 2.0],
            spatial: SpatialSettings {
                stereo_width: 1.15,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.30,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Vocal Clarity ---
        "vocal-clarity" | "vocal" | "voice" | "podcast" | "ax-voice" | "fx-voice" => Some(DspPreset {
            name: "vocal-clarity",
            description: "Voces y podcasts al frente con recorte de retumbes en graves y presencia limpia en rango medio",
            eq_bands: [-3.0, -2.5, -1.5, -0.5, 0.0, 0.5, 1.5, 2.0, 2.5, 2.5, 1.5, 1.0, 0.5, 0.0, 0.0],
            spatial: SpatialSettings {
                stereo_width: 1.05,
                binaural_crossfeed: false,
                crossfeed_level: 0.08,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Gaming 3D ---
        "gaming-3d" | "gaming" | "ax-gaming" | "fx-gaming" => Some(DspPreset {
            name: "gaming-3d",
            description: "Calibración Gaming 3D: detección espacial de pasos y detalles tácticos con explosiones de impacto",
            eq_bands: [3.0, 2.5, 1.5, 0.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.5, 3.0, 2.5, 2.0, 2.5, 2.5],
            spatial: SpatialSettings {
                stereo_width: 1.30,
                binaural_crossfeed: true,
                crossfeed_level: 0.15,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Concert Hall ---
        "concert-hall" | "arena" | "hall" => Some(DspPreset {
            name: "concert-hall",
            description: "Acústica envolvente de sala de conciertos con amplia apertura estéreo y agudos brillantes",
            eq_bands: [1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0, 3.0],
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.20,
                bass_exciter: false,
                bass_exciter_drive: 0.10,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- Nightcore ---
        "nightcore" => Some(DspPreset {
            name: "nightcore",
            description: "Curva V-Shape dinámica para música electrónica con graves contundentes y brillo enérgico",
            eq_bands: [3.5, 3.0, 2.0, 1.0, 0.0, -0.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            spatial: SpatialSettings {
                stereo_width: 1.20,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.25,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        // --- 8D Audio ---
        "8d" | "8d-audio" | "orbit" => Some(DspPreset {
            name: "8d",
            description: "Inmersión 8D Audio: rotación orbital continua a 0.15 Hz, apertura espacial y crossfeed binaural",
            eq_bands: [3.0, 2.5, 1.5, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 1.5, 2.0, 2.0, 2.5, 2.5],
            spatial: SpatialSettings {
                stereo_width: 1.28,
                binaural_crossfeed: true,
                crossfeed_level: 0.16,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
            modulation: ModulationSettings {
                rotation: true,
                rotation_speed_hz: 0.15,
                tremolo: false,
                tremolo_freq_hz: 4.0,
                tremolo_depth: 0.5,
                vibrato: false,
                vibrato_freq_hz: 5.0,
                vibrato_depth: 0.3,
            },
            loudness: LoudnessSettings::default(),
        }),

        // --- Tremolo ---
        "tremolo" => Some(DspPreset {
            name: "tremolo",
            description: "Modulación clásica de amplitud Tremolo a 4.5 Hz con ecualización cálida",
            eq_bands: [1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 1.5],
            spatial: SpatialSettings::default(),
            modulation: ModulationSettings {
                rotation: false,
                rotation_speed_hz: 0.15,
                tremolo: true,
                tremolo_freq_hz: 4.5,
                tremolo_depth: 0.55,
                vibrato: false,
                vibrato_freq_hz: 5.0,
                vibrato_depth: 0.3,
            },
            loudness: LoudnessSettings::default(),
        }),

        // --- Vibrato ---
        "vibrato" => Some(DspPreset {
            name: "vibrato",
            description: "Modulación analógica de tono Vibrato a 5.0 Hz con calidez sutil",
            eq_bands: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 1.5, 2.0, 2.0],
            spatial: SpatialSettings::default(),
            modulation: ModulationSettings {
                rotation: false,
                rotation_speed_hz: 0.15,
                tremolo: false,
                tremolo_freq_hz: 4.0,
                tremolo_depth: 0.5,
                vibrato: true,
                vibrato_freq_hz: 5.0,
                vibrato_depth: 0.35,
            },
            loudness: LoudnessSettings::default(),
        }),

        // --- Broadcast / EBU R128 Loudness Normalized ---
        "broadcast" | "radio" | "ebur128" | "normalize" | "normalized" => Some(DspPreset {
            name: "broadcast",
            description: "Transmisión broadcast EBU R128: nivel de sonoridad objetivo a -14.0 LUFS con control anti-pumping y ecualización de estudio",
            eq_bands: [1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0, 1.5, 1.5, 1.5],
            spatial: SpatialSettings {
                stereo_width: 1.10,
                binaural_crossfeed: true,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.12,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
        }),

        // --- Flat / Reset / Bypass ---
        "flat" | "off" | "reset" | "bypass" => Some(DspPreset {
            name: "flat",
            description: "Audio bit-perfect neutro sin alteración de señal ni procesamiento",
            eq_bands: [0.0; 15],
            spatial: SpatialSettings {
                stereo_width: 1.0,
                binaural_crossfeed: false,
                crossfeed_level: 0.0,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
        }),

        _ => None,
    }
}

pub fn apply_preset(name: &str, eq: &mut Equalizer, spatial: &mut SpatialProcessor) -> Result<String, String> {
    let preset = get_preset(name).ok_or_else(|| {
        format!(
            "Preset '{}' desconocido. Presets disponibles: music, bass-boost-3d, cinema-3d (dolby-atmos), vocal-clarity, gaming-3d, concert-hall, nightcore, 8d, tremolo, vibrato, broadcast (ebur128), flat",
            name
        )
    })?;

    for (band, &gain) in preset.eq_bands.iter().enumerate() {
        eq.set_band_db(band, gain);
    }
    spatial.set_settings(preset.spatial);

    Ok(format!("{}: {}", preset.name, preset.description))
}

pub fn apply_preset_to_params(name: &str, params: &mut super::DspParams) -> Result<String, String> {
    let preset = get_preset(name).ok_or_else(|| {
        format!(
            "Preset '{}' desconocido. Presets disponibles: music, bass-boost-3d, cinema-3d (dolby-atmos), vocal-clarity, gaming-3d, concert-hall, nightcore, 8d, tremolo, vibrato, broadcast (ebur128), flat",
            name
        )
    })?;

    params.gains_db = preset.eq_bands;
    params.spatial = preset.spatial;
    params.modulation = preset.modulation;
    params.loudness = preset.loudness;
    params.active_preset = Some(preset.name.to_string());
    params.version += 1;

    Ok(format!("{}: {}", preset.name, preset.description))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presets_exist() {
        let preset_names = [
            "music", "bass-boost-3d", "cinema-3d", "dolby-atmos",
            "vocal-clarity", "gaming-3d", "concert-hall", "nightcore", "8d", "tremolo", "vibrato", "broadcast", "flat",
        ];
        for name in preset_names {
            assert!(get_preset(name).is_some(), "Preset {} not found", name);
        }
    }

    #[test]
    fn test_apply_preset() {
        let mut eq = Equalizer::new(48000.0);
        let mut sp = SpatialProcessor::new(48000.0);

        let res = apply_preset("music", &mut eq, &mut sp);
        assert!(res.is_ok());
        assert!(eq.is_active);
        assert!(sp.is_active());

        let res_flat = apply_preset("flat", &mut eq, &mut sp);
        assert!(res_flat.is_ok());
        assert!(!eq.is_active);
        assert!(!sp.is_active());
    }
}
