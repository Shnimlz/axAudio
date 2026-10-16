//! Presets de ecualización y procesamiento 3D calibrados para audiófilos

use super::equalizer::Equalizer;
use super::spatial::{SpatialProcessor, SpatialSettings};

pub struct DspPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub eq_bands: [f32; 15], // dB por banda
    pub spatial: SpatialSettings,
}

pub fn get_preset(name: &str) -> Option<DspPreset> {
    match name.to_lowercase().as_str() {
        "dolby-atmos" | "cinema-3d" | "atmos" => Some(DspPreset {
            name: "cinema-3d",
            description: "Inmersión cinemática 3D tipo Dolby Atmos con crossfeed binaural y graves limpios",
            // 25, 40, 63, 100, 160, 250, 400, 630, 1k, 1.6k, 2.5k, 4k, 6.3k, 10k, 16k
            eq_bands: [3.0, 2.5, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 1.5, 2.0, 2.0, 2.5],
            spatial: SpatialSettings {
                stereo_width: 1.25,
                binaural_crossfeed: true,
                crossfeed_level: 0.18,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
        }),

        "bass-boost-3d" | "bass-boost" | "bass" => Some(DspPreset {
            name: "bass-boost-3d",
            description: "Realce profundo de sub-graves con armónicos limpios y escena ensanchada",
            eq_bands: [5.0, 4.0, 3.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0],
            spatial: SpatialSettings {
                stereo_width: 1.15,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.25,
            },
        }),

        "concert-hall" | "arena" | "hall" => Some(DspPreset {
            name: "concert-hall",
            description: "Acústica de sala de conciertos, apertura espacial amplia y aire en agudos",
            eq_bands: [1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0, 3.0],
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.22,
                bass_exciter: false,
                bass_exciter_drive: 0.10,
            },
        }),

        "vocal-clarity" | "vocal" | "voice" => Some(DspPreset {
            name: "vocal-clarity",
            description: "Voces cristalinas al frente con recorte de retumbes graves",
            eq_bands: [-2.5, -2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 2.5, 2.5, 1.5, 0.5, 0.0, 0.0, 0.0],
            spatial: SpatialSettings {
                stereo_width: 1.05,
                binaural_crossfeed: false,
                crossfeed_level: 0.08,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
        }),

        "nightcore" => Some(DspPreset {
            name: "nightcore",
            description: "Curva V-Shape dinámica para electrónica con graves definidos y brillo",
            eq_bands: [3.5, 3.0, 2.0, 1.0, 0.0, -0.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            spatial: SpatialSettings {
                stereo_width: 1.20,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.25,
            },
        }),

        "flat" | "off" | "reset" => Some(DspPreset {
            name: "flat",
            description: "Audio original bit-perfect sin procesamiento",
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
            "Preset '{}' desconocido. Presets disponibles: cinema-3d, dolby-atmos, bass-boost-3d, concert-hall, vocal-clarity, nightcore, flat",
            name
        )
    })?;

    for (band, &gain) in preset.eq_bands.iter().enumerate() {
        eq.set_band_db(band, gain);
    }
    spatial.set_settings(preset.spatial);

    Ok(format!("{}: {}", preset.name, preset.description))
}
