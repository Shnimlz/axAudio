//! Presets de ecualización y procesamiento 3D calibrados para audiófilos
//!
//! Curvas de respuesta acústica de estudio, ampliación Mid/Side y sonido envolvente.

use super::equalizer::Equalizer;
use super::spatial::{SpatialProcessor, SpatialSettings};

pub struct DspPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub eq_bands: [f32; 15], // dB por banda (25Hz a 16kHz)
    pub spatial: SpatialSettings,
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
        }),

        _ => None,
    }
}

pub fn apply_preset(name: &str, eq: &mut Equalizer, spatial: &mut SpatialProcessor) -> Result<String, String> {
    let preset = get_preset(name).ok_or_else(|| {
        format!(
            "Preset '{}' desconocido. Presets disponibles: music, bass-boost-3d, cinema-3d (dolby-atmos), vocal-clarity, gaming-3d, concert-hall, nightcore, flat",
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
            "Preset '{}' desconocido. Presets disponibles: music, bass-boost-3d, cinema-3d (dolby-atmos), vocal-clarity, gaming-3d, concert-hall, nightcore, flat",
            name
        )
    })?;

    params.gains_db = preset.eq_bands;
    params.spatial = preset.spatial;
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
            "vocal-clarity", "gaming-3d", "concert-hall", "nightcore", "flat",
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
