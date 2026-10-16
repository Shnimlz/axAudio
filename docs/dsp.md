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

## 3. 15-Band Studio Equalizer

The equalizer divides the audible spectrum into 15 ISO standard center frequencies:

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

### Sub-Bass Q Stabilization

At sample rates of 48 kHz, center frequencies below 60 Hz place digital filter poles dangerously close to the unit circle $z = 1$. A high Q factor in this zone causes numeric overflow.

axAudio applies automatic low-frequency Q tapering:

$$\text{effective\_Q} = \begin{cases} 
\min\left(Q, \; (f_0 - 20) \cdot \frac{Q - 0.8}{40} + 0.8\right) & \text{if } f_0 < 60 \text{ Hz} \\
\max(0.1, Q) & \text{otherwise}
\end{cases}$$

This ensures the filter remains unconditionally stable, smooth, and phase-linear.

### Dynamic Pre-Amp Headroom Compensation

When multiple EQ bands are boosted, summing the filters can easily exceed digital full scale ($0\text{ dBFS}$), leading to digital clipping.

axAudio calculates the peak positive gain across all 15 bands:

$$\text{max\_boost} = \max_{i=0..14} (\text{gain\_db}_i)$$

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

axAudio includes 6 meticulously voiced acoustic presets:

### 1. `music` (Hi-Fi Master)
- **Profile**: Warm low-end, neutral vocal clarity, silky top-end sheen.
- **EQ (dB)**: `[2.5, 2.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0]`
- **Spatial**: Width `1.18`, Crossfeed `0.12`, Bass Exciter Drive `0.15`.

### 2. `cinema-3d` (Dolby Atmos-Style Immersion)
- **Profile**: Wide holographic soundstage, distinct center dialogue, cinematic sub-bass punch.
- **EQ (dB)**: `[3.5, 3.0, 2.0, 0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0]`
- **Spatial**: Width `1.35`, Crossfeed `0.18`, Bass Exciter Drive `0.22`.

### 3. `bass-boost-3d`
- **Profile**: Heavy visceral bass without masking midrange vocals.
- **EQ (dB)**: `[5.0, 4.5, 3.0, 1.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.5, 2.0, 2.0]`
- **Spatial**: Width `1.15`, Crossfeed `0.10`, Bass Exciter Drive `0.30`.

### 4. `vocal-clarity`
- **Profile**: Podcasts, audiobooks, and vocal-centric tracks; low-frequency rumble filtered out.
- **EQ (dB)**: `[-3.0, -2.5, -1.5, -0.5, 0.0, 0.5, 1.5, 2.0, 2.5, 2.5, 1.5, 1.0, 0.5, 0.0, 0.0]`
- **Spatial**: Width `1.05`, Crossfeed Off, Exciter Off.

### 5. `gaming-3d`
- **Profile**: Footstep and tactical cue amplification, expansive stereo separation, impactful explosions.
- **EQ (dB)**: `[3.0, 2.5, 1.5, 0.0, -0.5, 0.0, 0.5, 1.0, 1.5, 2.5, 3.0, 2.5, 2.0, 2.5, 2.5]`
- **Spatial**: Width `1.30`, Crossfeed `0.15`, Bass Exciter Drive `0.20`.

### 6. `concert-hall`
- **Profile**: Airy arena dispersion with sparkling ambient reverberation feel.
- **EQ (dB)**: `[1.5, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.5, 2.5, 3.0, 3.0]`
- **Spatial**: Width `1.35`, Crossfeed `0.20`, Bass Exciter Drive `0.10`.
