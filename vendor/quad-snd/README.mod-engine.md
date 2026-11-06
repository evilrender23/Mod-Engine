# quad-snd 0.2.8 (vendored, patched)

Copy of [quad-snd](https://github.com/not-fl3/quad-snd) 0.2.8 (MIT/Apache-2.0, by Fedor
Logachev), used by macroquad, with one addition for Mod Engine:

- `quad_snd::set_mix_hook(Some(Box::new(|buf: &mut [f32]| ...)))`: a function called from
  the audio thread for every output buffer (interleaved stereo f32, 44100 Hz). Mod Engine's
  software mixer (`crates/modengine/src/audio.rs`: pitch, pan, fades, groups, intro→loop,
  positional sound) mixes its voices there, so it shares macroquad's single audio device on
  Linux (ALSA), Windows (WASAPI), macOS (CoreAudio) and Android (OpenSL ES).

The workspace uses it through `[patch.crates-io]` in the root `Cargo.toml`.
