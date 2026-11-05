//! Tiny procedural sound generator, so a game can declare retro sound effects
//! in a `.toml` file without shipping audio files. Produces 16-bit mono WAV bytes.

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SynthDef {
    /// "square", "saw", "triangle", "sine" or "noise"
    pub wave: String,
    /// Start frequency in Hz.
    pub freq: f64,
    /// End frequency in Hz (linear slide). Defaults to `freq`.
    pub freq_end: Option<f64>,
    /// Seconds.
    pub duration: f64,
    pub volume: f64,
    /// Seconds of fade-in.
    pub attack: f64,
    /// Seconds of fade-out at the end.
    pub release: f64,
    /// Duty cycle for square waves (0..1).
    pub duty: f64,
    /// Vibrato depth (fraction of frequency) and speed (Hz).
    pub vibrato: f64,
    pub vibrato_speed: f64,
    /// Repeat the tone this many times with a gap (arpeggio-like blips).
    pub repeat: u32,
    pub gap: f64,
    /// Frequency multiplier applied on each repeat (e.g. 1.5 for rising blips).
    pub repeat_pitch: f64,
}

impl Default for SynthDef {
    fn default() -> Self {
        SynthDef {
            wave: "square".into(),
            freq: 440.0,
            freq_end: None,
            duration: 0.15,
            volume: 0.4,
            attack: 0.005,
            release: 0.05,
            duty: 0.5,
            vibrato: 0.0,
            vibrato_speed: 8.0,
            repeat: 1,
            gap: 0.0,
            repeat_pitch: 1.0,
        }
    }
}

const RATE: u32 = 44_100;

pub fn render_wav(def: &SynthDef) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    let mut seed: u32 = 0x1234_5678;
    let reps = def.repeat.max(1);
    let mut pitch = 1.0;
    for _ in 0..reps {
        let n = (def.duration.max(0.001) * RATE as f64) as usize;
        let f0 = def.freq * pitch;
        let f1 = def.freq_end.unwrap_or(def.freq) * pitch;
        let mut phase = 0.0f64;
        let mut noise_val = 0.0f64;
        for i in 0..n {
            let t = i as f64 / RATE as f64;
            let k = i as f64 / n as f64;
            let mut f = f0 + (f1 - f0) * k;
            if def.vibrato > 0.0 {
                f *= 1.0 + def.vibrato * (t * def.vibrato_speed * std::f64::consts::TAU).sin();
            }
            let prev_phase = phase;
            phase = (phase + f / RATE as f64).fract();
            let s = match def.wave.as_str() {
                "sine" => (phase * std::f64::consts::TAU).sin(),
                "saw" => 2.0 * phase - 1.0,
                "triangle" => 1.0 - 4.0 * (phase - 0.5).abs(),
                "noise" => {
                    if phase < prev_phase {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        noise_val = (seed as f64 / u32::MAX as f64) * 2.0 - 1.0;
                    }
                    noise_val
                }
                _ => {
                    if phase < def.duty.clamp(0.05, 0.95) {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            let mut env = 1.0;
            if def.attack > 0.0 && t < def.attack {
                env = t / def.attack;
            }
            let remaining = def.duration - t;
            if def.release > 0.0 && remaining < def.release {
                env *= (remaining / def.release).max(0.0);
            }
            let v = (s * env * def.volume.clamp(0.0, 1.0) * 32767.0 * 0.8) as i16;
            samples.push(v);
        }
        let gap = (def.gap.max(0.0) * RATE as f64) as usize;
        samples.extend(std::iter::repeat_n(0, gap));
        pitch *= def.repeat_pitch;
    }
    wav_bytes(&samples)
}

fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}
