# Digital Signal Processing (DSP) Engine

This specification details the mathematical models, algorithmic design, filter topologies, and psychoacoustic principles powering the **axAudio** real-time DSP engine.

axAudio executes all signal processing directly in 32-bit floating-point planar buffers (`&mut [f32]`), achieving professional studio fidelity with microsecond latency.

---

## 1. Master DSP Signal Flow

The audio processing pipeline operates in four sequential stages inside the hot decoding loop:

```
                  RAW PCM INPUT (Planar Stereo f32)
                                │
                                ▼
         ┌──────────────────────────────────────────────┐
         │ 1. Dynamic Headroom Compensation (Pre-amp)   │
         │    attenuation = 10^(-max_boost_db / 20)     │
         └──────────────────────┬───────────────────────┘
                                │
                                ▼
         ┌──────────────────────────────────────────────┐
         │ 2. 15-Band TDF-II Biquad Graphic Equalizer   │
         │    ISO Frequencies: 25 Hz to 16 kHz          │
         │    Calibrated Frequency-Dependent Q Factors  │
         └──────────────────────┬───────────────────────┘
                                │
                                ▼
         ┌──────────────────────────────────────────────┐
         │ 3. Spatializer & Psychoacoustic Engine       │
         │    • Harmonic Sub-Bass Exciter (85 Hz tanh)  │
         │    • Mid/Side 3D Widening (Phase-Coherent)   │
         │    • Natural Acoustic Crossfeed (700 Hz LP)  │
         └──────────────────────┬───────────────────────┘
                                │
                                ▼
         ┌──────────────────────────────────────────────┐
         │ 4. Transparent Peak Soft-Limiter             │
         │    Linear below 0.95 (-0.45 dBFS)            │
         │    Hyperbolic Tangent (tanh) Transient Clamp │
         └──────────────────────┬───────────────────────┘
                                │
                                ▼
                     TO OPUS ENCODER & RTP UDP
```

---

## 2. Transposed Direct Form II (TDF-II) Biquad Filters

All filtering in axAudio is built upon second-order Infinite Impulse Response (IIR) biquad sections implementing the audio equalization formulas derived from Robert Bristow-Johnson's *Audio EQ Cookbook*.

### Why Transposed Direct Form II?

Traditional implementations frequently rely on **Direct Form I** (DF-I), which requires four delay registers ($x[n-1], x[n-2], y[n-1], y[n-2]$). In single-precision floating-point arithmetic (`f32`), DF-I suffers from catastrophic cancellation errors in low-frequency recursive poles, producing audible quantization noise, background hiss, and static.

axAudio implements **Transposed Direct Form II (TDF-II)**:

```
             x[n] ─────────┬──────── (b0) ───────────────────(+)────► y[n]
                           │                                  ▲
                           ├──────── (b1) ─────(+)─── [z^-1] ─┤ (s1)
                           │                    ▲             │
                           │                    │    (-a1) ───┤
                           │                    │             │
                           └──────── (b2) ─────(+)─── [z^-1] ─┘ (s2)
                                                ▲
                                                │    (-a2) ───┘
```

#### Difference Equations

$$\begin{aligned}
y[n] &= b_0 \cdot x[n] + s_1[n-1] \\
s_1[n] &= b_1 \cdot x[n] - a_1 \cdot y[n] + s_2[n-1] \\
s_2[n] &= b_2 \cdot x[n] - a_2 \cdot y[n]
\end{aligned}$$

Where $s_1$ and $s_2$ are the canonical state registers.

#### Advantages of TDF-II:
1. **Minimal Memory State**: Only two internal state registers per channel instead of four.
2. **Superior Dynamic Range**: Accumulation occurs inside the integrators, suppressing quantization noise by over 18 dB in low-frequency bands compared to DF-I.
3. **Anti-Denormal Flushing**: Single-precision CPU calculations can stall when numbers approach zero (denormals/subnormals). axAudio flushes outputs smaller than $10^{-15}$ to exact zero:
   ```rust
   let clean_out = if out.abs() < 1e-15 { 0.0 } else { out };
   ```

---

## 3. Multi-Mode Studio Equalizer

axAudio features an adaptable, multi-topology equalization engine capable of switching dynamically between ISO graphic modes, hardware-calibrated critical band modes, and arbitrary parametric Mid/Side stages.

### 3.1 15-Band ISO Graphic Equalizer

The standard 15-band equalizer divides the audible spectrum into ISO center frequencies with frequency-dependent Q calibration:

| Band | Center Freq | Calibrated Q | Acoustic Target |
| :---: | :---: | :---: | :--- |
| **0** | **25 Hz** | `0.8` | Sub-bass rumble; low Q avoids boomy resonant ringing. |
| **1** | **40 Hz** | `1.0` | Deep kick drum and synthetic bass fundament. |
| **2** | **63 Hz** | `1.2` | Electric bass weight and punch. |
| **3** | **100 Hz** | `1.4` | Low-end body and warmth. |
| **4** | **160 Hz** | `1.6` | Upper bass warmth and fullness. |
| **5** | **250 Hz** | `1.8` | Lower midrange clarity; reduces muddiness. |
| **6** | **400 Hz** | `1.8` | Midrange body and snare punch. |
| **7** | **630 Hz** | `1.8` | Vocal core and horn fundamentals. |
| **8** | **1.0 kHz** | `1.8` | Vocal projection and speech intelligibility. |
| **9** | **1.6 kHz** | `1.8` | String attack and vocal bite. |
| **10** | **2.5 kHz** | `1.8` | High midrange presence and percussion snap. |
| **11** | **4.0 kHz** | `1.5` | Vocal articulation and edge. |
| **12** | **6.3 kHz** | `1.5` | Sibilance and cymbal definitions. |
| **13** | **10.0 kHz**| `1.2` | High treble brilliance. |
| **14** | **16.0 kHz**| `1.2` | Airy sheen and high-frequency sparkle. |

---

### 3.2 Dolby DAX 20 Psychoacoustic Critical Bands

Extracted directly from Motorola / Dolby Audio Processing (DAP) DAX v3 hardware configurations, this topology aligns filters with the human auditory system's **Bark critical bandwidths**:

$$\text{Bark} = 13 \cdot \arctan(0.00076 \cdot f) + 3.5 \cdot \arctan\left(\left(\frac{f}{7500}\right)^2\right)$$

The 20 critical band center frequencies are:
`[47, 141, 234, 328, 422, 562, 750, 984, 1266, 1641, 2109, 2719, 3469, 4406, 5578, 7078, 9000, 11484, 14766, 19688] Hz`

- **Logarithmic Spacing**: Denser resolution in the human speech region (300 Hz - 4 kHz) to accurately match the ear's cochlear filter banks.
- **Constant Psychoacoustic Q ($Q \approx 2.5$)**: Provides surgical correction without phase smearing across neighboring critical bands.

---

### 3.3 ISO 31-Band 1/3-Octave Studio Standard

For acoustically challenging room correction and master-grade frequency response adjustments, axAudio provides a full 1/3-octave 31-band bank:

`[20, 25, 31.5, 40, 50, 63, 80, 100, 125, 160, 200, 250, 315, 400, 500, 630, 800, 1000, 1250, 1600, 2000, 2500, 3150, 4000, 5000, 6300, 8000, 10000, 12500, 16000, 20000] Hz`

Each band is calibrated with a constant fractional bandwidth of:
$$Q = \frac{\sqrt{2^{1/3}}}{2^{1/3} - 1} \approx 4.318$$

---

### 3.4 Dynamic Parametric Equalizer & Orthogonal Mid/Side (M/S) Matrix

In parametric mode, arbitrary dynamic filter bands can be allocated at runtime. Each band allows independent configuration of:
- **Frequency ($f_0$)**: 20 Hz to 22,000 Hz.
- **Bandwidth ($Q$)**: 0.1 to 20.0.
- **Gain**: -24.0 dB to +24.0 dB.
- **Filter Curve**: Peaking, LowShelf, HighShelf, LowPass, HighPass, Notch, BandPass.
- **Channel Routing**: `Stereo`, `Mid`, or `Side`.

#### Orthogonal Mid/Side Decomposition

By decomposing incoming stereo signals into sum (mono / center) and difference (stereo side / ambience) components, axAudio performs surgical corrections without collateral damage to stereo imaging:

1. **Unitary Forward M/S Encoding**:
   $$\begin{aligned}
   M[n] &= \frac{L[n] + R[n]}{\sqrt{2}} \\
   S[n] &= \frac{L[n] - R[n]}{\sqrt{2}}
   \end{aligned}$$
2. **Channel-Isolated Filtering**:
   - `Mid` filters process only $M[n]$ (lead vocals, kick drum, bass, dialogue).
   - `Side` filters process only $S[n]$ (room reverb, wide guitars, stereo synthesizers, binaural atmosphere).
   - `Stereo` filters process both channels in planar stereo space.
3. **Unitary Inverse M/S Decoding**:
   $$\begin{aligned}
   L[n] &= \frac{M[n] + S[n]}{\sqrt{2}} \\
   R[n] &= \frac{M[n] - S[n]}{\sqrt{2}}
   \end{aligned}$$

The $\frac{1}{\sqrt{2}}$ factor preserves total acoustic energy across encoding and decoding with mathematical exactness:
$$|L|^2 + |R|^2 = |M|^2 + |S|^2$$

---

### 3.5 Biquad Filter Topologies

axAudio implements 7 distinct filter transfer functions derived from Robert Bristow-Johnson's audio biquad models:

1. **Peaking**: Bell-shaped boost/cut centered at $f_0$.
2. **LowShelf**: In-phase boost/cut for all frequencies below $f_0$.
3. **HighShelf**: In-phase boost/cut for all frequencies above $f_0$.
4. **LowPass (Butterworth 2nd-order)**: 12 dB/octave attenuation above $f_0$ ($Q = \frac{1}{\sqrt{2}} \approx 0.707$).
5. **HighPass (Butterworth 2nd-order)**: 12 dB/octave attenuation below $f_0$ ($Q = 0.707$).
6. **Notch (Band-Reject)**: Infinite theoretical attenuation at $f_0$ with zero gain alteration elsewhere. Essential for removing ground hum (50/60 Hz) or microphonic feedback.
7. **BandPass**: Constant 0 dB peak gain at $f_0$ with 6 dB/octave roll-off on either side.

---

### 3.6 Sub-Bass Q Stabilization & Dynamic Pre-Amp Headroom Compensation

#### Sub-Bass Q Stabilization

At 48 kHz sampling rates, center frequencies below 60 Hz place digital filter poles dangerously close to the unit circle $z = 1$. A high Q factor in this zone causes numeric overflow and ringing.

axAudio applies automatic low-frequency Q tapering:

$$\text{effective\_Q} = \begin{cases} 
\min\left(Q, \; (f_0 - 20) \cdot \frac{Q - 0.8}{40} + 0.8\right) & \text{if } f_0 < 60 \text{ Hz} \\
\max(0.1, Q) & \text{otherwise}
\end{cases}$$

#### Dynamic Pre-Amp Headroom Compensation

When multiple EQ bands are boosted, summing the filters can easily exceed digital full scale ($0\text{ dBFS}$), leading to digital clipping.

axAudio calculates the peak positive gain across all active equalizer bands:

$$\text{max\_boost} = \max_{i} (\text{gain\_db}_i)$$

If $\text{max\_boost} > 0.2\text{ dB}$, an automatic pre-attenuation gain is applied to the input block:

$$\text{preamp} = 10^{-\frac{\text{max\_boost}}{20}}$$

This guarantees **zero digital clipping** regardless of extreme equalizer boosts.

---

## 4. Phase-Coherent Mid/Side 3D Spatializer

Conventional surround plugins often use delay lines or phase inversion to widen the stereo field. On Discord (where users listen via headphones or phone speakers), delay lines cause severe **comb filtering**, making music sound hollow, metallic, or phase-cancelled.

axAudio uses a **Phase-Coherent Mid/Side (M/S) Matrix**:

```
Input Left  ──┬──(+)── (*0.5) ──► Mid (Center)  ── (*gain_mid)  ──┬──(+)──► Output Left
              │   │                                               │   ▲
Input Right ──┴──(-)── (*0.5) ──► Side (Stereo) ── (*gain_side) ─┬┴───│───► Output Right
                                                                 └────(-)
```

### Mathematical Transformation

$$\begin{aligned}
\text{Mid} &= \frac{L + R}{2} \\
\text{Side} &= \frac{L - R}{2}
\end{aligned}$$

The spatial processor modifies the stereo balance according to the target stereo width $W \in [0.0, 2.0]$:

- **$W = 0.0$**: Pure mono collapse ($L = R = \text{Mid}$).
- **$W = 1.0$**: Neutral, unaltered stereo field.
- **$W = 1.35$**: Standard immersive 3D stage width.
- **$W = 2.0$**: Ultra-wide cinematic dispersion.

### Center Dialogue Preservation

Expanding the Side channel ($W > 1.0$) typically pulls vocals and lead instruments backward. axAudio compensates by adjusting the Mid channel gain:

$$g_{\text{mid}} = \max\left(0.5, \; 1.0 - (W - 1.0) \cdot 0.18\right), \quad g_{\text{side}} = W$$

$$\begin{aligned}
L_{\text{out}} &= \text{Mid} \cdot g_{\text{mid}} + \text{Side} \cdot g_{\text{side}} \\
R_{\text{out}} &= \text{Mid} \cdot g_{\text{mid}} - \text{Side} \cdot g_{\text{side}}
\end{aligned}$$

Because this relies exclusively on linear matrix algebra, the phase relationship remains completely intact with **zero comb-filtering artifacts**.

---

## 5. Natural Acoustic Crossfeed (Head-Shadow Diffraction)

Headphone listening isolates the left and right channels entirely. In the physical world, sound from a right speaker reaches the left ear attenuated and low-pass filtered by the acoustic shadow of the human head.

axAudio emulates this physical phenomenon using a single-pole lowpass filter tuned to 700 Hz:

$$H_{\text{head}}(z) = \frac{a_0}{1 + b_1 z^{-1}}$$

The crossfeed blending equation:

$$\begin{aligned}
L_{\text{cross}} &= L \cdot \left(1 - \frac{C}{2}\right) + H_{\text{head}}(R) \cdot C \\
R_{\text{cross}} &= R \cdot \left(1 - \frac{C}{2}\right) + H_{\text{head}}(L) \cdot C
\end{aligned}$$

Where $C \in [0.0, 0.40]$ represents the crossfeed blend level (default: `0.18`). This transforms sterile stereo separation into an organic, fatigue-free soundstage.

---

## 6. Psychoacoustic Sub-Bass Exciter

Earphones and smaller laptop speakers struggle to physically reproduce frequencies below 50 Hz. Attempting to boost these frequencies with raw EQ only causes driver distortion and muddiness.

axAudio includes an intentional harmonic exciter:

1. **Isolation**: A second-order Butterworth lowpass biquad isolates frequencies below 85 Hz:
   $$x_{\text{sub}} = \text{Biquad}_{\text{LP85}}(x)$$
2. **Harmonic Generation**: Passing the sub-bass signal through a hyperbolic tangent generates clean, musically warm second and third-order harmonics:
   $$x_{\text{harm}} = \tanh(1.5 \cdot x_{\text{sub}})$$
3. **Injection**: The generated harmonics are mixed back with the original signal at a controlled drive level $D \in [0.0, 1.0]$:
   $$x_{\text{out}} = x + x_{\text{harm}} \cdot D \cdot 0.15$$

This tricks the human brain's auditory cortex into perceiving deep visceral bass even on hardware unable to reproduce true sub-bass.

---

## 7. Transparent Soft-Clip Peak Limiter

To handle instantaneous transients while avoiding harsh digital brickwall clipping, axAudio applies an inline analog-style soft-limiting function:

$$\text{soft\_limit}(x) = \begin{cases}
x & \text{if } |x| \le 0.95 \\
0.95 + 0.05 \cdot \tanh\left(\frac{x - 0.95}{0.05}\right) & \text{if } x > 0.95 \\
-0.95 - 0.05 \cdot \tanh\left(\frac{-x - 0.95}{0.05}\right) & \text{if } x < -0.95
\end{cases}$$

- **Below 0.95 (-0.45 dBFS)**: The signal passes with bit-perfect linearity and zero harmonic alteration.
- **Above 0.95**: Transients are rounded smoothly, preserving musical punch without square-wave distortion.

---

## 8. Calibrated Factory Presets

axAudio includes 18 meticulously voiced acoustic presets divided across hardware-calibrated Dolby profiles, audiophile mastering curves, and creative effects:

### 8.1 Motorola Dolby Atmos Hardware Profiles

Extracted directly via ADB root from Motorola moto g85 5G DAX v3 hardware configurations:

#### 1. `moto-spatial` (aliases: `dolby-moto`, `moto-buds`, `spatial-audio`)
- **Acoustic Target**: Calibrated to Motorola Moto Buds Bass. Surround virtualizer +6 dB, crossfeed 45°, Mid-boost on vocal presence, Side-boost on high-frequency air, and sub-bass harmonic excitation.
- **Parametric EQ**:
  - Sub-Bass (35 Hz, Q=0.8, +4.5 dB, Peaking, Stereo)
  - Punch Bass (80 Hz, Q=1.2, +3.0 dB, Peaking, Stereo)
  - Mid Vocal Focus (1.0 kHz, Q=1.4, +2.5 dB, Peaking, Mid channel)
  - Upper Mid Presence (3.2 kHz, Q=2.0, +2.0 dB, Peaking, Stereo)
  - Spatial Air Sheen (12.0 kHz, Q=0.7, +3.5 dB, HighShelf, Side channel)
- **Spatial**: Width `1.25`, Crossfeed `0.18` (binaural enabled), Bass Exciter `0.25`.

#### 2. `dolby-music` (alias: `dolby-audio-music`)
- **Acoustic Target**: Dolby Audio DAP v3 Music profile. Intelligent Equalizer (IEQ) warmth curve, natural harmonic bass enhancement, and subtle holographic soundstage widening.
- **Parametric EQ**:
  - Low Shelf (60 Hz, Q=0.7, +2.5 dB, LowShelf, Stereo)
  - Lower Mid Warmth (250 Hz, Q=1.0, +1.0 dB, Peaking, Stereo)
  - Vocal Intelligibility (2.0 kHz, Q=1.5, +1.5 dB, Peaking, Mid channel)
  - Treble Brilliance (10.0 kHz, Q=0.8, +2.0 dB, HighShelf, Side channel)
- **Spatial**: Width `1.18`, Crossfeed `0.12`, Bass Exciter `0.15`.

#### 3. `dolby-movie` (alias: `dolby-cinema`)
- **Acoustic Target**: Dolby Audio DAP v3 Movie / Cinema profile. Focuses dialogue intelligibility onto the center Mid channel while expanding atmospheric sound effects across the Side channel.
- **Parametric EQ**:
  - Sub-Bass Boom (45 Hz, Q=1.0, +3.5 dB, Peaking, Stereo)
  - Dialogue Core (1.2 kHz, Q=1.8, +3.5 dB, Peaking, Mid channel)
  - Dialogue Articulation (3.5 kHz, Q=2.0, +2.5 dB, Peaking, Mid channel)
  - Cinema Surround Width (8.0 kHz, Q=0.7, +3.0 dB, HighShelf, Side channel)
- **Spatial**: Width `1.35`, Crossfeed `0.18`, Bass Exciter `0.22`.

#### 4. `dolby-game` (alias: `dolby-gaming`)
- **Acoustic Target**: Dolby Audio DAP v3 Gaming profile. Tactical acoustic cue boosts (footsteps and reloads at 2.5k–6kHz) with punchy explosion dynamics.
- **Parametric EQ**:
  - Sub Impact (50 Hz, Q=1.2, +4.0 dB, Peaking, Stereo)
  - Mud Cut (300 Hz, Q=1.5, -2.0 dB, Peaking, Stereo)
  - Footstep / Reload Snap (2.8 kHz, Q=2.5, +3.5 dB, Peaking, Stereo)
  - Spatial Positioning Cue (5.5 kHz, Q=2.0, +3.0 dB, Peaking, Side channel)
- **Spatial**: Width `1.30`, Crossfeed `0.15`, Bass Exciter `0.20`.

#### 5. `dolby-voice` (aliases: `dolby-dialogue`, `dolby-podcast`)
- **Acoustic Target**: Dolby Audio Dialogue Enhancer. Center-channel speech focus with low-end rumble attenuation and sibilance notch filtering.
- **Parametric EQ**:
  - Rumble HighPass (70 Hz, Q=0.707, HighPass, Stereo)
  - Chest Body (220 Hz, Q=1.2, +1.5 dB, Peaking, Mid channel)
  - Vocal Intelligibility (1.5 kHz, Q=1.5, +3.5 dB, Peaking, Mid channel)
  - Speech Articulation (3.5 kHz, Q=2.0, +2.5 dB, Peaking, Mid channel)
  - De-Esser Taming (7.0 kHz, Q=4.0, -2.5 dB, Peaking, Mid channel)
- **Spatial**: Width `1.0` (pure centered focus), Crossfeed Off, Exciter Off.

---

### 8.2 Audiophile Mastering Profiles

#### 6. `music` (alias: `hifi`)
- **Profile**: Warm low-end, neutral vocal clarity, silky top-end sheen.
- **EQ (15-Band dB)**: `[2.5, 2.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0]`
- **Spatial**: Width `1.18`, Crossfeed `0.12`, Bass Exciter `0.15`.

#### 7. `cinema-3d` (alias: `atmos`)
- **Profile**: Wide holographic soundstage, distinct center dialogue, cinematic sub-bass punch.
- **EQ (15-Band dB)**: `[3.5, 3.0, 2.0, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0]`
- **Spatial**: Width `1.35`, Crossfeed `0.18`, Bass Exciter `0.22`.

#### 8. `bass-boost-3d` (aliases: `bass-boost`, `bass`)
- **Profile**: Heavy visceral bass without masking midrange vocals.
- **EQ (15-Band dB)**: `[5.0, 4.5, 3.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.5, 2.0, 2.0]`
- **Spatial**: Width `1.15`, Crossfeed `0.10`, Bass Exciter `0.30`.

#### 9. `vocal-clarity` (alias: `vocal`)
- **Profile**: Podcasts and vocal-centric tracks; low-frequency rumble filtered out.
- **EQ (15-Band dB)**: `[-3.0, -2.5, -1.5, -0.5, 0.0, 0.5, 1.5, 2.0, 2.5, 2.5, 1.5, 1.0, 0.5, 0.0, 0.0]`
- **Spatial**: Width `1.05`, Crossfeed Off, Exciter Off.

#### 10. `gaming-3d`
- **Profile**: Footstep and tactical cue amplification with expansive stereo separation.
- **EQ (15-Band dB)**: `[3.0, 2.5, 1.5, 0.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.5, 3.0, 2.5, 2.0, 2.5, 2.5]`
- **Spatial**: Width `1.30`, Crossfeed `0.15`, Bass Exciter `0.20`.

#### 11. `concert-hall`
- **Profile**: Airy arena dispersion with sparkling ambient reverberation feel.
- **EQ (15-Band dB)**: `[1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0, 3.0]`
- **Spatial**: Width `1.35`, Crossfeed `0.20`, Bass Exciter `0.10`.

---

### 8.3 Acoustic Effects & Normalization Profiles

#### 12. `nightcore`
- **Profile**: High-energy dance aesthetic; 1.25x speed and pitch acceleration with attenuated low rumble.
- **Timescale**: Speed `1.25`, Pitch `1.25`.
- **EQ**: Low frequencies attenuated (-2.0 dB below 100 Hz), high frequencies boosted (+2.5 dB at 8k–16kHz).

#### 13. `vaporwave`
- **Profile**: Nostalgic analog aesthetic; slowed 0.85x speed and pitch with warm low-pass rolloff and subtle stereo widening.
- **Timescale**: Speed `0.85`, Pitch `0.85`.
- **Spatial**: Width `1.20`.

#### 14. `8d`
- **Profile**: Continuous 360° binaural circular orbit around the listener's head.
- **Modulation**: Rotation active at `0.12 Hz` (~8.3 seconds per full rotation).
- **Spatial**: Width `1.35`, Crossfeed `0.20`.

#### 15. `tremolo`
- **Profile**: Rhythmic amplitude modulation creating classic vintage amplifier tremolo.
- **Modulation**: Tremolo active at `4.0 Hz` with `0.4` depth.

#### 16. `vibrato`
- **Profile**: Musical frequency pitch modulation for expressive vocal or synth warble.
- **Modulation**: Vibrato active at `5.0 Hz` with `0.3` semitone pitch depth.

#### 17. `broadcast`
- **Profile**: ITU-R BS.1770-4 and EBU R128 loudness normalization calibrated for competitive streaming and multi-track listening sessions.
- **Loudness**: Target `-14.0 LUFS`, max boost `+9.0 dB`, max attenuation `-12.0 dB`.

#### 18. `flat` (aliases: `reset`, `bypass`)
- **Profile**: Completely bypasses all EQ, spatialization, modulation, and timescale stages, passing bit-perfect raw decoded PCM.
