//! Presets de ecualización y procesamiento 3D calibrados para audiófilos
//!
//! Curvas de respuesta acústica de estudio, perfiles Dolby Atmos extraídos de hardware Motorola,
//! ampliación Mid/Side, sonido envolvente 3D y normalización EBU R128.

use super::equalizer::{Equalizer, EqChannel, ParametricBand};
use super::loudness::LoudnessSettings;
use super::modulation::ModulationSettings;
use super::spatial::{SpatialProcessor, SpatialSettings};
use super::timescale::TimescaleSettings;

#[derive(Clone)]
pub struct DspPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub eq_bands: [f32; 15], // dB por banda (25Hz a 16kHz)
    pub parametric_bands: Option<Vec<ParametricBand>>,
    pub spatial: SpatialSettings,
    pub modulation: ModulationSettings,
    pub loudness: LoudnessSettings,
    pub timescale: TimescaleSettings,
}

pub fn get_preset(name: &str) -> Option<DspPreset> {
    match name.to_lowercase().replace('_', "-").as_str() {
        // --- Motorola Dolby Atmos (Extraído vía ADB de moto buds bass) ---
        "moto-spatial" | "dolby-moto" | "moto-buds" | "spatial-audio" => Some(DspPreset {
            name: "moto-spatial",
            description: "Motorola Dolby Atmos Spatial Audio calibrado vía ADB (moto buds bass): virtualizador envolvente +6dB, crossfeed 45°, excitación armónica de sub-grave (30-90Hz) y realce M/S de vocales y aire 3D",
            // 25, 40, 63, 100, 160, 250, 400, 630, 1k, 1.6k, 2.5k, 4k, 6.3k, 10k, 16k
            eq_bands: [2.5, 2.0, 1.8, 1.5, 1.2, 1.0, 1.0, 1.2, 1.8, 2.2, 2.3, 2.3, 2.0, 1.0, -0.5],
            parametric_bands: Some(vec![
                ParametricBand {
                    freq: 2250.0,
                    gain_db: 1.8,
                    q: 1.8,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
                ParametricBand {
                    freq: 11250.0,
                    gain_db: 2.2,
                    q: 1.2,
                    filter_type: "high_shelf".to_string(),
                    channel: EqChannel::Side,
                },
                ParametricBand {
                    freq: 70.0,
                    gain_db: 0.0,
                    q: 0.707,
                    filter_type: "high_pass".to_string(),
                    channel: EqChannel::Side,
                },
            ]),
            spatial: SpatialSettings {
                stereo_width: 1.40,
                binaural_crossfeed: true,
                crossfeed_level: 0.18,
                bass_exciter: true,
                bass_exciter_drive: 0.28,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
        }),

        // --- Dolby Audio DAP v3 Music ---
        "dolby-music" | "dolby-audio-music" => Some(DspPreset {
            name: "dolby-music",
            description: "Dolby Audio DAP v3 Music: curva IEQ cálida, realce armónico natural de bajos y apertura holográfica moderada",
            eq_bands: [2.0, 1.8, 1.5, 1.0, 0.5, 0.0, 0.0, 0.5, 0.8, 1.2, 1.5, 1.5, 1.8, 1.2, 0.0],
            parametric_bands: Some(vec![
                ParametricBand {
                    freq: 1688.0,
                    gain_db: 1.0,
                    q: 1.4,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
                ParametricBand {
                    freq: 9000.0,
                    gain_db: 1.5,
                    q: 1.2,
                    filter_type: "high_shelf".to_string(),
                    channel: EqChannel::Side,
                },
            ]),
            spatial: SpatialSettings {
                stereo_width: 1.22,
                binaural_crossfeed: true,
                crossfeed_level: 0.12,
                bass_exciter: true,
                bass_exciter_drive: 0.18,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
        }),

        // --- Dolby Audio DAP v3 Movie / Cinema ---
        "dolby-movie" | "dolby-cinema" => Some(DspPreset {
            name: "dolby-movie",
            description: "Dolby Audio DAP v3 Movie: máxima inteligibilidad de diálogos centrales con Mid-boost quirúrgico y campo surround amplio",
            eq_bands: [3.5, 3.0, 2.2, 1.0, 0.0, 0.0, 0.8, 1.2, 2.0, 2.5, 2.5, 2.0, 2.2, 2.5, 2.0],
            parametric_bands: Some(vec![
                ParametricBand {
                    freq: 2250.0,
                    gain_db: 2.5,
                    q: 2.0,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
                ParametricBand {
                    freq: 141.0,
                    gain_db: 1.5,
                    q: 1.0,
                    filter_type: "low_shelf".to_string(),
                    channel: EqChannel::Side,
                },
            ]),
            spatial: SpatialSettings {
                stereo_width: 1.45,
                binaural_crossfeed: true,
                crossfeed_level: 0.22,
                bass_exciter: true,
                bass_exciter_drive: 0.25,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
        }),

        // --- Dolby Audio DAP v3 Game ---
        "dolby-game" | "dolby-gaming" => Some(DspPreset {
            name: "dolby-game",
            description: "Dolby Audio DAP v3 Game: presencia táctica de pasos y recargas (2.5k-6k), localización tridimensional y pegada de explosiones",
            eq_bands: [2.5, 2.0, 1.0, 0.0, -0.5, 0.0, 0.5, 1.0, 2.0, 3.0, 3.2, 2.8, 2.5, 2.2, 2.0],
            parametric_bands: Some(vec![
                ParametricBand {
                    freq: 3750.0,
                    gain_db: 2.2,
                    q: 2.5,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
                ParametricBand {
                    freq: 7125.0,
                    gain_db: 2.0,
                    q: 1.5,
                    filter_type: "high_shelf".to_string(),
                    channel: EqChannel::Side,
                },
            ]),
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.18,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
        }),

        // --- Dolby Audio Dialogue Enhancer ---
        "dolby-voice" | "dolby-dialogue" | "dolby-podcast" => Some(DspPreset {
            name: "dolby-voice",
            description: "Dolby Audio Dialogue Enhancer: realce central enfático de voces limpiando retumbes y sibilancias",
            eq_bands: [-4.0, -3.0, -2.0, -0.5, 0.0, 0.5, 1.8, 2.5, 3.0, 3.0, 2.2, 1.5, 0.5, 0.0, -1.0],
            parametric_bands: Some(vec![
                ParametricBand {
                    freq: 1313.0,
                    gain_db: 2.0,
                    q: 1.5,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
                ParametricBand {
                    freq: 3000.0,
                    gain_db: 2.0,
                    q: 1.8,
                    filter_type: "peaking".to_string(),
                    channel: EqChannel::Mid,
                },
            ]),
            spatial: SpatialSettings {
                stereo_width: 1.05,
                binaural_crossfeed: false,
                crossfeed_level: 0.05,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings {
                enabled: true,
                target_lufs: -14.0,
                max_gain_db: 9.0,
                min_gain_db: -12.0,
            },
            timescale: TimescaleSettings::default(),
        }),

        // --- Music / Hi-Fi ---
        "music" | "hifi" | "ax-music" | "fx-music" => Some(DspPreset {
            name: "music",
            description: "Calibración Hi-Fi: graves cálidos y profundos, medios transparentes y agudos sedosos sin fatiga auditiva",
            eq_bands: [2.5, 2.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.18,
                binaural_crossfeed: true,
                crossfeed_level: 0.12,
                bass_exciter: true,
                bass_exciter_drive: 0.15,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Cinema 3D / Dolby Atmos Legacy ---
        "cinema-3d" | "dolby-atmos" | "atmos" | "cinema" | "movie" | "fx-cinema" => Some(DspPreset {
            name: "cinema-3d",
            description: "Inmersión cinemática 3D tipo Dolby Atmos con ampliación Mid/Side, diálogos cristalinos y sub-graves profundos",
            eq_bands: [3.5, 3.0, 2.0, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.18,
                bass_exciter: true,
                bass_exciter_drive: 0.22,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Bass Boost 3D ---
        "bass-boost-3d" | "bass-boost" | "bass" | "ax-bass" | "fx-bass" => Some(DspPreset {
            name: "bass-boost-3d",
            description: "Realce visceral de sub-graves con armónicos limpios y escena ampliada sin distorsión áspera",
            eq_bands: [5.0, 4.5, 3.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.5, 2.0, 2.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.15,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.30,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Vocal Clarity ---
        "vocal-clarity" | "vocal" | "voice" | "podcast" | "ax-voice" | "fx-voice" => Some(DspPreset {
            name: "vocal-clarity",
            description: "Voces y podcasts al frente con recorte de retumbes en graves y presencia limpia en rango medio",
            eq_bands: [-3.0, -2.5, -1.5, -0.5, 0.0, 0.5, 1.5, 2.0, 2.5, 2.5, 1.5, 1.0, 0.5, 0.0, 0.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.05,
                binaural_crossfeed: false,
                crossfeed_level: 0.08,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Gaming 3D ---
        "gaming-3d" | "gaming" | "ax-gaming" | "fx-gaming" => Some(DspPreset {
            name: "gaming-3d",
            description: "Calibración Gaming 3D: detección espacial de pasos y detalles tácticos con explosiones de impacto",
            eq_bands: [3.0, 2.5, 1.5, 0.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.5, 3.0, 2.5, 2.0, 2.5, 2.5],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.30,
                binaural_crossfeed: true,
                crossfeed_level: 0.15,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Concert Hall ---
        "concert-hall" | "arena" | "hall" => Some(DspPreset {
            name: "concert-hall",
            description: "Acústica envolvente de sala de conciertos con amplia apertura estéreo y agudos brillantes",
            eq_bands: [1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0, 3.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.35,
                binaural_crossfeed: true,
                crossfeed_level: 0.20,
                bass_exciter: false,
                bass_exciter_drive: 0.10,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Nightcore ---
        "nightcore" => Some(DspPreset {
            name: "nightcore",
            description: "Efecto Nightcore real: velocidad y tono acelerados (+25% pitch y tempo con remuestreador Hermite) y EQ V-Shape electrónica",
            eq_bands: [3.5, 3.0, 2.0, 1.0, 0.0, -0.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.20,
                binaural_crossfeed: false,
                crossfeed_level: 0.10,
                bass_exciter: true,
                bass_exciter_drive: 0.25,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings {
                speed: 1.25,
                pitch: 1.25,
                rate: 1.0,
            },
        }),

        // --- Vaporwave ---
        "vaporwave" | "slowed" | "slowed-reverb" => Some(DspPreset {
            name: "vaporwave",
            description: "Efecto Vaporwave real: tempo y tono ralentizados (-18% pitch y tempo analógico con remuestreador Hermite) y amplitud espacial",
            eq_bands: [2.0, 2.5, 2.0, 1.0, 0.5, 0.0, -0.5, -1.0, -1.5, -1.5, -1.0, -0.5, 0.0, 0.5, 1.0],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.30,
                binaural_crossfeed: true,
                crossfeed_level: 0.15,
                bass_exciter: true,
                bass_exciter_drive: 0.20,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings {
                speed: 0.82,
                pitch: 0.82,
                rate: 1.0,
            },
        }),

        // --- 8D Audio ---
        "8d" | "8d-audio" | "orbit" => Some(DspPreset {
            name: "8d",
            description: "Inmersión 8D Audio: rotación orbital continua a 0.15 Hz, apertura espacial y crossfeed binaural",
            eq_bands: [3.0, 2.5, 1.5, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 1.5, 2.0, 2.0, 2.5, 2.5],
            parametric_bands: None,
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
                flanger: false,
                flanger_freq_hz: 0.4,
                flanger_depth: 0.6,
                flanger_feedback: 0.4,
                chorus: false,
                chorus_freq_hz: 0.8,
                chorus_depth: 0.5,
                chorus_mix: 0.5,
                phaser: false,
                phaser_freq_hz: 0.2,
                phaser_depth: 0.5,
                phaser_mix: 0.5,
            },
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Tremolo ---
        "tremolo" => Some(DspPreset {
            name: "tremolo",
            description: "Modulación clásica de amplitud Tremolo a 4.5 Hz con ecualización cálida",
            eq_bands: [1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 1.5],
            parametric_bands: None,
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
                flanger: false,
                flanger_freq_hz: 0.4,
                flanger_depth: 0.6,
                flanger_feedback: 0.4,
                chorus: false,
                chorus_freq_hz: 0.8,
                chorus_depth: 0.5,
                chorus_mix: 0.5,
                phaser: false,
                phaser_freq_hz: 0.2,
                phaser_depth: 0.5,
                phaser_mix: 0.5,
            },
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Vibrato ---
        "vibrato" => Some(DspPreset {
            name: "vibrato",
            description: "Modulación analógica de tono Vibrato a 5.0 Hz con calidez sutil",
            eq_bands: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 1.5, 2.0, 2.0],
            parametric_bands: None,
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
                flanger: false,
                flanger_freq_hz: 0.4,
                flanger_depth: 0.6,
                flanger_feedback: 0.4,
                chorus: false,
                chorus_freq_hz: 0.8,
                chorus_depth: 0.5,
                chorus_mix: 0.5,
                phaser: false,
                phaser_freq_hz: 0.2,
                phaser_depth: 0.5,
                phaser_mix: 0.5,
            },
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        // --- Broadcast / EBU R128 Loudness Normalized ---
        "broadcast" | "radio" | "ebur128" | "normalize" | "normalized" => Some(DspPreset {
            name: "broadcast",
            description: "Transmisión broadcast EBU R128: nivel de sonoridad objetivo a -14.0 LUFS con control anti-pumping y ecualización de estudio",
            eq_bands: [1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0, 1.5, 1.5, 1.5],
            parametric_bands: None,
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
            timescale: TimescaleSettings::default(),
        }),

        // --- Flat / Reset / Bypass ---
        "flat" | "off" | "reset" | "bypass" => Some(DspPreset {
            name: "flat",
            description: "Audio bit-perfect neutro sin alteración de señal ni procesamiento",
            eq_bands: [0.0; 15],
            parametric_bands: None,
            spatial: SpatialSettings {
                stereo_width: 1.0,
                binaural_crossfeed: false,
                crossfeed_level: 0.0,
                bass_exciter: false,
                bass_exciter_drive: 0.0,
            },
            modulation: ModulationSettings::default(),
            loudness: LoudnessSettings::default(),
            timescale: TimescaleSettings::default(),
        }),

        _ => None,
    }
}

pub fn list_builtin_presets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("moto-spatial", "Motorola Dolby Atmos Spatial Audio (moto buds bass)"),
        ("dolby-music", "Dolby Audio DAP v3 Music (calidez IEQ y realce natural)"),
        ("dolby-movie", "Dolby Audio DAP v3 Movie (diálogos nítidos y surround 3D)"),
        ("dolby-game", "Dolby Audio DAP v3 Game (localización táctica y pegada)"),
        ("dolby-voice", "Dolby Audio Dialogue Enhancer (claridad vocal enfática)"),
        ("music", "Calibración Hi-Fi para audiófilos"),
        ("cinema-3d", "Inmersión cinemática 3D tipo Dolby Atmos"),
        ("bass-boost-3d", "Realce visceral de graves 3D con excitador armónico"),
        ("vocal-clarity", "Voces y podcasts al frente con presencia nítida"),
        ("gaming-3d", "Calibración Gaming 3D para localización táctica"),
        ("concert-hall", "Acústica envolvente de sala de conciertos"),
        ("nightcore", "Nightcore real (+25% pitch y velocidad)"),
        ("vaporwave", "Vaporwave real (-18% pitch y tempo con reverb)"),
        ("8d", "Efecto 8D rotacional orbital 360"),
        ("tremolo", "Modulación clásica de amplitud"),
        ("vibrato", "Modulación analógica de tono"),
        ("broadcast", "Normalización perceptual EBU R128 a -14 LUFS"),
        ("flat", "Bypass transparente bit-perfect"),
    ]
}

pub fn apply_preset(name: &str, eq: &mut Equalizer, spatial: &mut SpatialProcessor) -> Result<String, String> {
    let preset = get_preset(name).ok_or_else(|| {
        format!(
            "Preset '{}' desconocido. Presets disponibles: moto-spatial (dolby-moto), dolby-music, dolby-movie, dolby-game, dolby-voice, music, bass-boost-3d, cinema-3d, vocal-clarity, gaming-3d, concert-hall, nightcore, vaporwave, 8d, tremolo, vibrato, broadcast, flat",
            name
        )
    })?;

    for (band, &gain) in preset.eq_bands.iter().enumerate() {
        eq.set_band_db(band, gain);
    }
    if let Some(ref pb) = preset.parametric_bands {
        eq.set_parametric_bands(pb);
    } else {
        eq.clear_parametric_bands();
    }
    spatial.set_settings(preset.spatial);

    Ok(format!("{}: {}", preset.name, preset.description))
}

pub fn apply_preset_to_params(name: &str, params: &mut super::DspParams) -> Result<String, String> {
    let preset = get_preset(name).ok_or_else(|| {
        format!(
            "Preset '{}' desconocido. Presets disponibles: moto-spatial (dolby-moto), dolby-music, dolby-movie, dolby-game, dolby-voice, music, bass-boost-3d, cinema-3d, vocal-clarity, gaming-3d, concert-hall, nightcore, vaporwave, 8d, tremolo, vibrato, broadcast, flat",
            name
        )
    })?;

    params.gains_db = preset.eq_bands;
    params.parametric_bands = preset.parametric_bands.clone();
    params.spatial = preset.spatial;
    params.modulation = preset.modulation;
    params.loudness = preset.loudness;
    params.timescale = preset.timescale;
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
            "moto-spatial", "dolby-moto", "dolby-music", "dolby-movie", "dolby-game", "dolby-voice",
            "music", "bass-boost-3d", "cinema-3d", "dolby-atmos",
            "vocal-clarity", "gaming-3d", "concert-hall", "nightcore", "vaporwave", "8d", "tremolo", "vibrato", "broadcast", "flat",
        ];
        for name in preset_names {
            assert!(get_preset(name).is_some(), "Preset {} not found", name);
        }
    }

    #[test]
    fn test_apply_preset() {
        let mut eq = Equalizer::new(48000.0);
        let mut sp = SpatialProcessor::new(48000.0);

        let res = apply_preset("moto-spatial", &mut eq, &mut sp);
        assert!(res.is_ok());
        assert!(eq.is_active);
        assert!(eq.has_mid_side);
        assert!(sp.is_active());

        let res_flat = apply_preset("flat", &mut eq, &mut sp);
        assert!(res_flat.is_ok());
        assert!(!eq.is_active);
        assert!(!sp.is_active());
    }
}
