//! Software mixer: pitch, pan, fades, volume groups, intro→loop and positional sound.
//!
//! macroquad's audio (quad-snd) can only play a sound with a volume and a loop flag. The
//! game's sounds are mixed here instead: the backend hands the output buffer of quad-snd's
//! audio thread to [`Mixer::render`] (see `vendor/quad-snd`), and without a backend
//! (headless runs, tests) the engine advances the mixer by one step of time per tick, so
//! `audio_is_playing`, track positions and fades behave the same in tests.
//!
//! Output: interleaved stereo f32 at [`RATE`] Hz. Ramps (fades, group and master gain)
//! advance in blocks of [`BLOCK`] frames (1.5 ms).

use std::sync::{Arc, Mutex, MutexGuard};

pub const RATE: f64 = 44100.0;
pub const BLOCK: usize = 64;
pub const MAX_VOICES: usize = 128;

/// Extra data of a sound from its TOML.
#[derive(Default, Clone, Debug)]
pub struct SoundMeta {
    pub group: Option<String>,
    pub loop_start: Option<f64>,
    pub loop_end: Option<f64>,
    /// Encoded file played before the main one (then the main one loops).
    pub intro: Option<Vec<u8>>,
}

/// Decodes WAV / OGG into interleaved stereo f32 at 44100 Hz (linear resampling).
pub fn decode(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let mut r = audrey::Reader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("{e:?}"))?;
    let desc = r.description();
    let ch = desc.channel_count() as usize;
    let rate = desc.sample_rate() as f64;
    if ch == 0 || rate <= 0.0 {
        return Err("no channels".into());
    }
    let mut raw: Vec<f32> = Vec::new();
    for s in r.samples::<f32>() {
        raw.push(s.map_err(|e| format!("{e:?}"))?);
    }
    let frames = raw.len() / ch;
    let mut st = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let l = raw[f * ch];
        let r = if ch >= 2 { raw[f * ch + 1] } else { l };
        st.push(l);
        st.push(r);
    }
    if (rate - RATE).abs() < 0.5 {
        return Ok(st);
    }
    let out_frames = ((frames as f64) * RATE / rate).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let p = i as f64 * rate / RATE;
        let j = p.floor() as usize;
        let t = (p - j as f64) as f32;
        let k = (j + 1).min(frames.saturating_sub(1));
        for c in 0..2 {
            out.push(st[j.min(frames - 1) * 2 + c] * (1.0 - t) + st[k * 2 + c] * t);
        }
    }
    Ok(out)
}

/// A value that moves linearly to a target over time (fades).
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub cur: f32,
    pub target: f32,
    rate: f32,
}

impl Ramp {
    pub fn new(v: f32) -> Ramp {
        Ramp { cur: v, target: v, rate: 0.0 }
    }
    /// Go to `target` in `ms` milliseconds (0 = at once).
    pub fn set(&mut self, target: f32, ms: f64) {
        self.target = target;
        if ms <= 0.0 {
            self.cur = target;
            self.rate = 0.0;
        } else {
            self.rate = (target - self.cur).abs() / (ms * RATE / 1000.0) as f32;
        }
    }
    fn advance(&mut self, frames: usize) {
        if self.cur < self.target {
            self.cur = (self.cur + self.rate * frames as f32).min(self.target);
        } else if self.cur > self.target {
            self.cur = (self.cur - self.rate * frames as f32).max(self.target);
        }
    }
    fn moving(&self) -> bool {
        self.cur != self.target
    }
}

/// Distance attenuation (GameMaker's linear distance clamped model):
/// full volume up to `reference`, `1 - factor` at `max` and beyond.
#[derive(Clone, Copy, Debug)]
pub struct Falloff {
    pub reference: f64,
    pub max: f64,
    pub factor: f64,
}

impl Default for Falloff {
    fn default() -> Self {
        Falloff { reference: 100.0, max: 600.0, factor: 1.0 }
    }
}

impl Falloff {
    pub fn gain(&self, d: f64) -> f64 {
        if self.max <= self.reference {
            return if d <= self.reference { 1.0 } else { (1.0 - self.factor).max(0.0) };
        }
        let k = (d.clamp(self.reference, self.max) - self.reference) / (self.max - self.reference);
        (1.0 - self.factor * k).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug)]
pub struct Emitter {
    pub x: f64,
    pub y: f64,
    pub gain: f32,
    pub pitch: f32,
    pub falloff: Falloff,
}

#[derive(Clone, Copy, Debug)]
pub enum Spatial {
    None,
    At { x: f64, y: f64, falloff: Falloff },
    Emitter(u32),
}

#[derive(Clone, Debug)]
pub struct Voice {
    pub id: u32,
    pub snd: u32,
    /// Position in frames of the sound.
    pub pos: f64,
    pub pitch: f32,
    pub gain: Ramp,
    pub pan: f32,
    pub looped: bool,
    pub paused: bool,
    pub priority: f64,
    pub spatial: Spatial,
    /// Stop when a fade-out reaches 0.
    stop_at_zero: bool,
    /// Sound that follows without a gap, looped, when this one ends (intro→loop).
    pub then: Option<u32>,
    done: bool,
    /// Order of creation (voice stealing takes the oldest of the lowest priority).
    serial: u64,
}

pub struct Slot {
    pub name: String,
    pub pcm: Option<Arc<Vec<f32>>>,
    pub failed: bool,
    pub group: usize,
    /// `volume` of the TOML.
    pub volume: f32,
    /// audio_sound_gain(sound, ...) on the asset: every voice of it.
    pub gain: Ramp,
    pub meta_loop: (Option<f64>, Option<f64>),
    pub has_intro: bool,
    /// Frames where the loop starts / ends (computed when decoded).
    pub loop_frames: (usize, usize),
}

impl Slot {
    pub fn frames(&self) -> usize {
        self.pcm.as_ref().map_or(0, |p| p.len() / 2)
    }
}

pub struct Group {
    pub name: String,
    pub gain: Ramp,
}

/// Options of a new voice.
#[derive(Clone, Debug)]
pub struct PlayOpts {
    pub looped: bool,
    pub gain: f32,
    pub pitch: f32,
    pub pan: f32,
    /// Start position in seconds.
    pub offset: f64,
    pub priority: f64,
    pub spatial: Spatial,
    /// Fade in from silence, in milliseconds.
    pub fade_in: f64,
}

impl Default for PlayOpts {
    fn default() -> Self {
        PlayOpts { looped: false, gain: 1.0, pitch: 1.0, pan: 0.0, offset: 0.0, priority: 0.0, spatial: Spatial::None, fade_in: 0.0 }
    }
}

pub struct Mixer {
    pub slots: Vec<Slot>,
    pub voices: Vec<Voice>,
    pub groups: Vec<Group>,
    pub master: Ramp,
    pub listener: (f64, f64),
    pub emitters: Vec<Option<Emitter>>,
    /// True while an audio device pulls [`render`](Mixer::render); otherwise the engine
    /// advances the mixer by itself every step.
    pub realtime: bool,
    next_id: u32,
    serial: u64,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            slots: vec![],
            voices: vec![],
            groups: vec![Group { name: "default".into(), gain: Ramp::new(1.0) }],
            master: Ramp::new(1.0),
            listener: (0.0, 0.0),
            emitters: vec![],
            realtime: false,
            next_id: 0,
            serial: 0,
            scratch: vec![],
        }
    }
}

fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    // balance law: the centre keeps both channels at full volume
    (if p > 0.0 { 1.0 - p } else { 1.0 }, if p < 0.0 { 1.0 + p } else { 1.0 })
}

impl Mixer {
    /// Registers a sound (name, TOML volume and meta) — PCM comes later with [`set_pcm`].
    pub fn add_slot(&mut self, name: &str, volume: f64, meta: &SoundMeta) {
        let group = self.group(meta.group.as_deref().unwrap_or("default"));
        self.slots.push(Slot {
            name: name.to_string(),
            pcm: None,
            failed: false,
            group,
            volume: volume as f32,
            gain: Ramp::new(1.0),
            meta_loop: (meta.loop_start, meta.loop_end),
            has_intro: meta.intro.is_some(),
            loop_frames: (0, 0),
        });
    }

    /// Index of a group, created on first use.
    pub fn group(&mut self, name: &str) -> usize {
        match self.groups.iter().position(|g| g.name == name) {
            Some(i) => i,
            None => {
                self.groups.push(Group { name: name.to_string(), gain: Ramp::new(1.0) });
                self.groups.len() - 1
            }
        }
    }

    /// Decoded samples of sound `snd`; `intro_frames` = length of the intro part.
    pub fn set_pcm(&mut self, snd: u32, pcm: Vec<f32>, intro_frames: usize) {
        let Some(s) = self.slots.get_mut(snd as usize) else { return };
        let frames = pcm.len() / 2;
        let at = |sec: f64| ((sec * RATE).round().max(0.0) as usize).min(frames);
        let start = s.meta_loop.0.map(at).unwrap_or(if s.has_intro { intro_frames } else { 0 });
        let end = s.meta_loop.1.map(at).unwrap_or(frames);
        s.loop_frames = if end > start { (start, end) } else { (0, frames) };
        s.pcm = Some(Arc::new(pcm));
    }

    pub fn play(&mut self, snd: u32, o: PlayOpts) -> u32 {
        if self.voices.len() >= MAX_VOICES {
            // steal the lowest priority voice (the oldest among equals), if it is not above us
            if let Some((i, v)) = self.voices.iter().enumerate().min_by(|a, b| {
                a.1.priority.partial_cmp(&b.1.priority).unwrap_or(std::cmp::Ordering::Equal).then(a.1.serial.cmp(&b.1.serial))
            }) {
                if v.priority > o.priority {
                    return 0;
                }
                self.voices.remove(i);
            }
        }
        self.next_id += 1;
        self.serial += 1;
        let frames = self.slots.get(snd as usize).map_or(0, |s| s.frames()) as f64;
        let mut pos = (o.offset * RATE).max(0.0);
        if frames > 0.0 && pos >= frames {
            pos = if o.looped { pos % frames } else { frames };
        }
        let mut gain = Ramp::new(if o.fade_in > 0.0 { 0.0 } else { o.gain });
        if o.fade_in > 0.0 {
            gain.set(o.gain, o.fade_in);
        }
        self.voices.push(Voice {
            id: self.next_id,
            snd,
            pos,
            pitch: o.pitch.max(0.0),
            gain,
            pan: o.pan,
            looped: o.looped,
            paused: false,
            priority: o.priority,
            spatial: o.spatial,
            stop_at_zero: false,
            then: None,
            done: false,
            serial: self.serial,
        });
        self.next_id
    }

    pub fn voice(&self, id: u32) -> Option<&Voice> {
        self.voices.iter().find(|v| v.id == id)
    }
    pub fn voice_mut(&mut self, id: u32) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    /// Stops a voice, at once or fading out over `ms`.
    pub fn stop(&mut self, id: u32, ms: f64) {
        if ms <= 0.0 {
            self.voices.retain(|v| v.id != id);
        } else if let Some(v) = self.voice_mut(id) {
            v.gain.set(0.0, ms);
            v.stop_at_zero = true;
        }
    }
    pub fn stop_sound(&mut self, snd: u32, ms: f64) {
        let ids: Vec<u32> = self.voices.iter().filter(|v| v.snd == snd).map(|v| v.id).collect();
        for id in ids {
            self.stop(id, ms);
        }
    }
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }
    pub fn stop_group(&mut self, g: usize, ms: f64) {
        let ids: Vec<u32> = self.voices.iter().filter(|v| self.slots.get(v.snd as usize).is_some_and(|s| s.group == g)).map(|v| v.id).collect();
        for id in ids {
            self.stop(id, ms);
        }
    }

    /// Track position of a voice in seconds.
    pub fn position(&self, id: u32) -> Option<f64> {
        self.voice(id).map(|v| v.pos / RATE)
    }

    pub fn length(&self, snd: u32) -> f64 {
        self.slots.get(snd as usize).map_or(0.0, |s| s.frames() as f64 / RATE)
    }

    fn spatial(&self, sp: &Spatial) -> Option<(f32, f32, f32)> {
        let (lx, ly) = self.listener;
        let calc = |x: f64, y: f64, f: &Falloff| {
            let d = (x - lx).hypot(y - ly);
            let pan = ((x - lx) / f.max.max(1.0)).clamp(-1.0, 1.0);
            (f.gain(d) as f32, pan as f32)
        };
        match sp {
            Spatial::None => Some((1.0, 0.0, 1.0)),
            Spatial::At { x, y, falloff } => {
                let (g, p) = calc(*x, *y, falloff);
                Some((g, p, 1.0))
            }
            Spatial::Emitter(e) => {
                let em = self.emitters.get(*e as usize)?.as_ref()?;
                let (g, p) = calc(em.x, em.y, &em.falloff);
                Some((g * em.gain, p, em.pitch))
            }
        }
    }

    /// Mixes every voice into `out` (interleaved stereo; it is added to what is there)
    /// and advances time by `out.len() / 2` frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            let block = &mut out[done * 2..(done + n) * 2];
            self.render_block(block, n);
            done += n;
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Advances time without output (no audio device).
    pub fn advance(&mut self, frames: usize) {
        let mut buf = std::mem::take(&mut self.scratch);
        buf.clear();
        buf.resize(frames * 2, 0.0);
        self.render(&mut buf);
        self.scratch = buf;
    }

    /// Mixes frames `f0..n` of voice `vi`. Returns Some(frame) if it switched to its
    /// follow-up sound at that frame (the rest is mixed with the new sound).
    fn mix_voice(&mut self, vi: usize, out: &mut [f32], f0: usize, n: usize) -> Option<usize> {
        let master = self.master.cur;
        let v = &self.voices[vi];
        if v.paused || v.done {
            return None;
        }
        let Some(slot) = self.slots.get(v.snd as usize) else {
            self.voices[vi].done = true;
            return None;
        };
        let Some(pcm) = slot.pcm.clone() else {
            if slot.failed {
                self.voices[vi].done = true;
            }
            return None;
        };
        let Some((sg, span, spitch)) = self.spatial(&v.spatial) else {
            // its emitter was freed
            self.voices[vi].done = true;
            return None;
        };
        let g = master * self.groups[slot.group].gain.cur * slot.gain.cur * slot.volume * v.gain.cur * sg;
        let (gl, gr) = pan_gains(v.pan + span);
        let (gl, gr) = (gl * g, gr * g);
        let step = (v.pitch * spitch) as f64;
        let total = pcm.len() / 2;
        let (ls, le) = if v.looped { slot.loop_frames } else { (0, total) };
        let le = if le == 0 { total } else { le };
        let v = &mut self.voices[vi];
        let mut pos = v.pos;
        for f in f0..n {
            if pos >= le as f64 {
                if v.looped && le > ls {
                    pos = ls as f64 + (pos - le as f64) % (le - ls) as f64;
                } else if let Some(next) = v.then.take() {
                    // the follow-up starts where the intro's last sample left off
                    v.pos = pos - le as f64;
                    v.snd = next;
                    v.looped = true;
                    return Some(f);
                } else {
                    v.done = true;
                    break;
                }
            }
            let i = pos as usize;
            let t = (pos - i as f64) as f32;
            let j = if i + 1 < le { i + 1 } else if v.looped { ls } else { i };
            let (a0, a1) = (pcm[i * 2], pcm[i * 2 + 1]);
            let (b0, b1) = if j < total { (pcm[j * 2], pcm[j * 2 + 1]) } else { (a0, a1) };
            out[f * 2] += (a0 + (b0 - a0) * t) * gl;
            out[f * 2 + 1] += (a1 + (b1 - a1) * t) * gr;
            pos += step;
        }
        v.pos = pos;
        None
    }

    fn render_block(&mut self, out: &mut [f32], n: usize) {
        for vi in 0..self.voices.len() {
            let mut f0 = 0;
            // a voice can change sound half-way through the block (intro→loop)
            while f0 < n {
                match self.mix_voice(vi, out, f0, n) {
                    Some(f) => f0 = f,
                    None => break,
                }
            }
        }
        // time moves on for the ramps
        self.master.advance(n);
        for gr in &mut self.groups {
            gr.gain.advance(n);
        }
        for s in &mut self.slots {
            if s.gain.moving() {
                s.gain.advance(n);
            }
        }
        for v in &mut self.voices {
            if !v.paused {
                v.gain.advance(n);
                if v.stop_at_zero && v.gain.cur <= 0.0 {
                    v.done = true;
                }
            }
        }
        if self.voices.iter().any(|v| v.done) {
            self.voices.retain(|v| !v.done);
        }
    }
}

// ------------------------------------------------------------------ engine side

/// The mixer shared with the audio thread.
#[derive(Default)]
pub struct AudioState {
    mixer: Arc<Mutex<Mixer>>,
    synced: bool,
    /// audio_listener_position() was called: the listener no longer follows the camera.
    pub listener_set: bool,
}

/// Locks the mixer, registering the project's sounds on first use.
pub fn lock(e: &mut crate::engine::Engine) -> MutexGuard<'_, Mixer> {
    if !e.audio_mix.synced {
        e.audio_mix.synced = true;
        let mut m = e.audio_mix.mixer.lock().unwrap_or_else(|p| p.into_inner());
        for s in &e.sounds {
            m.add_slot(&s.name, s.volume, &s.meta);
        }
    }
    e.audio_mix.mixer.lock().unwrap_or_else(|p| p.into_inner())
}

/// The mixer for the backend's audio thread.
pub fn handle(e: &mut crate::engine::Engine) -> Arc<Mutex<Mixer>> {
    drop(lock(e));
    e.audio_mix.mixer.clone()
}

/// Decodes sound `snd` if it is not yet (headless runs decode on first play).
pub fn ensure_decoded(e: &mut crate::engine::Engine, snd: u32) {
    {
        let m = lock(e);
        match m.slots.get(snd as usize) {
            Some(s) if s.pcm.is_none() && !s.failed => {}
            _ => return,
        }
    }
    let Some(asset) = e.sounds.get(snd as usize) else { return };
    let name = asset.name.clone();
    let main = decode(&asset.data);
    let intro = asset.meta.intro.as_ref().map(|b| decode(b));
    let result = match (main, intro) {
        (Ok(m), None) => Ok((m, 0)),
        (Ok(m), Some(Ok(mut i))) => {
            let n = i.len() / 2;
            i.extend_from_slice(&m);
            Ok((i, n))
        }
        (Err(err), _) | (_, Some(Err(err))) => Err(err),
    };
    let mut m = lock(e);
    match result {
        Ok((pcm, intro)) => m.set_pcm(snd, pcm, intro),
        Err(err) => {
            eprintln!("[error] sound '{name}': {err}");
            if let Some(s) = m.slots.get_mut(snd as usize) {
                s.failed = true;
            }
        }
    }
}

/// Decodes every sound (the backend does it at start, like before).
pub fn decode_all(e: &mut crate::engine::Engine) {
    for i in 0..e.sounds.len() {
        ensure_decoded(e, i as u32);
    }
}

pub fn stop_all(e: &mut crate::engine::Engine) {
    lock(e).stop_all();
}

/// Once per step: the listener follows the centre of the view (unless placed by
/// audio_listener_position) and, without an audio device, time advances one step.
pub fn step(e: &mut crate::engine::Engine) {
    if e.sounds.is_empty() {
        return;
    }
    let follow = !e.audio_mix.listener_set;
    let centre = (e.cam.x + e.cam.w / 2.0, e.cam.y + e.cam.h / 2.0);
    let frames = (RATE / e.step_rate()).round() as usize;
    let mut m = lock(e);
    if follow {
        m.listener = centre;
    }
    if !m.realtime && !m.voices.is_empty() {
        m.advance(frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono sine wave as interleaved stereo PCM.
    fn sine(freq: f64, secs: f64) -> Vec<f32> {
        let n = (secs * RATE) as usize;
        (0..n).flat_map(|i| {
            let s = (i as f64 / RATE * freq * std::f64::consts::TAU).sin() as f32 * 0.5;
            [s, s]
        })
        .collect()
    }

    fn mixer_with(pcm: Vec<f32>, meta: SoundMeta, intro: usize) -> Mixer {
        let mut m = Mixer::default();
        m.add_slot("s", 1.0, &meta);
        m.set_pcm(0, pcm, intro);
        m
    }

    fn crossings(buf: &[f32]) -> usize {
        buf.chunks(2).map(|c| c[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn pitch_two_doubles_the_frequency() {
        let mut m = mixer_with(sine(441.0, 2.0), SoundMeta::default(), 0);
        m.play(0, PlayOpts::default());
        let mut a = vec![0.0; 44100 * 2];
        m.render(&mut a[..]);
        let mut m = mixer_with(sine(441.0, 2.0), SoundMeta::default(), 0);
        m.play(0, PlayOpts { pitch: 2.0, ..Default::default() });
        let mut b = vec![0.0; 44100 * 2];
        // half a second: at double speed the 2 s sound lasts 1 s
        m.render(&mut b[..44100]);
        let (ca, cb) = (crossings(&a), crossings(&b[..44100]));
        assert!((ca as i64 - 441).abs() <= 1, "{ca}");
        assert!((cb as i64 - 441).abs() <= 1, "pitch 2 gives 882 Hz: {cb} cycles in 0.5 s");
        // and it ends after 1 s instead of 2
        m.render(&mut b[44100..]);
        assert!(m.voices.is_empty() || m.voices[0].pos >= 44100.0 * 1.99);
    }

    #[test]
    fn fades_are_linear_over_the_given_time() {
        let pcm = vec![0.5; 44100 * 4]; // DC, 2 s
        let mut m = mixer_with(pcm, SoundMeta::default(), 0);
        let id = m.play(0, PlayOpts::default());
        m.voice_mut(id).unwrap().gain.set(0.0, 1000.0);
        let mut buf = vec![0.0; 44100 * 2];
        m.render(&mut buf);
        let at = |t: f64| buf[((t * RATE) as usize) * 2];
        assert!((at(0.0) - 0.5).abs() < 0.01);
        assert!((at(0.25) - 0.375).abs() < 0.01, "{}", at(0.25));
        assert!((at(0.5) - 0.25).abs() < 0.01, "{}", at(0.5));
        assert!(at(0.999) < 0.01);
        // a fade-out stop removes the voice when silent
        let mut m = mixer_with(vec![0.5; 44100 * 4], SoundMeta::default(), 0);
        let id = m.play(0, PlayOpts::default());
        m.stop(id, 100.0);
        m.advance(4410 + BLOCK);
        assert!(m.voice(id).is_none());
    }

    #[test]
    fn pan_moves_the_sound_between_channels() {
        let pcm = vec![0.5; 2000];
        let mut m = mixer_with(pcm.clone(), SoundMeta::default(), 0);
        m.play(0, PlayOpts { pan: 1.0, ..Default::default() });
        let mut buf = vec![0.0; 200];
        m.render(&mut buf);
        assert_eq!((buf[10], buf[11]), (0.0, 0.5), "pan 1 = right only");
        let mut m = mixer_with(pcm, SoundMeta::default(), 0);
        m.play(0, PlayOpts { pan: -0.5, ..Default::default() });
        let mut buf = vec![0.0; 200];
        m.render(&mut buf);
        assert_eq!((buf[10], buf[11]), (0.5, 0.25));
    }

    #[test]
    fn groups_master_and_asset_gain_multiply() {
        let mut m = Mixer::default();
        m.add_slot("mus", 0.5, &SoundMeta { group: Some("musica".into()), ..Default::default() });
        m.set_pcm(0, vec![1.0; 2000], 0);
        m.play(0, PlayOpts { looped: true, ..Default::default() });
        let g = m.group("musica");
        m.groups[g].gain.set(0.5, 0.0);
        m.master.set(0.5, 0.0);
        let mut buf = vec![0.0; 100];
        m.render(&mut buf);
        assert!((buf[0] - 0.125).abs() < 1e-6, "{}", buf[0]);
        // group fade to silence over 10 ms
        m.groups[g].gain.set(0.0, 10.0);
        m.advance(441 + BLOCK);
        let mut buf = vec![0.0; 100];
        m.render(&mut buf);
        assert_eq!(buf[0], 0.0);
        m.stop_group(g, 0.0);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn intro_plays_once_then_the_loop_repeats() {
        // intro = 100 frames of 0.1, loop = 50 frames of 0.9
        let mut pcm = vec![0.1; 200];
        pcm.extend(vec![0.9; 100]);
        let mut m = mixer_with(pcm, SoundMeta { intro: Some(vec![]), ..Default::default() }, 100);
        assert_eq!(m.slots[0].loop_frames, (100, 150));
        m.play(0, PlayOpts { looped: true, ..Default::default() });
        let mut buf = vec![0.0; 2 * 400];
        m.render(&mut buf);
        assert!((buf[2 * 50] - 0.1).abs() < 1e-6);
        for f in [120, 160, 230, 399] {
            assert!((buf[2 * f] - 0.9).abs() < 1e-6, "frame {f}: {}", buf[2 * f]);
        }
        assert_eq!(m.voices.len(), 1, "still looping");
        // loop_start / loop_end in seconds inside a single file
        let mut m = mixer_with(vec![0.2; 2 * 44100], SoundMeta { loop_start: Some(0.25), loop_end: Some(0.5), ..Default::default() }, 0);
        assert_eq!(m.slots[0].loop_frames, (11025, 22050));
        let id = m.play(0, PlayOpts { looped: true, ..Default::default() });
        m.advance(30000);
        let p = m.position(id).unwrap();
        assert!((0.25..0.5).contains(&p), "{p}");
        // not looped: plays to the end and stops
        let mut m = mixer_with(vec![0.2; 2 * 1000], SoundMeta { loop_start: Some(0.001), ..Default::default() }, 0);
        m.play(0, PlayOpts::default());
        m.advance(1001);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn a_voice_can_chain_into_a_looping_sound_without_a_gap() {
        let mut m = Mixer::default();
        m.add_slot("intro", 1.0, &SoundMeta::default());
        m.add_slot("loop", 1.0, &SoundMeta::default());
        m.set_pcm(0, vec![0.1; 2 * 100], 0);
        m.set_pcm(1, vec![0.9; 2 * 30], 0);
        let id = m.play(0, PlayOpts::default());
        m.voice_mut(id).unwrap().then = Some(1);
        let mut buf = vec![0.0; 2 * 300];
        m.render(&mut buf);
        assert!((buf[2 * 99] - 0.1).abs() < 1e-6);
        for f in 100..300 {
            assert!((buf[2 * f] - 0.9).abs() < 1e-6, "frame {f} = {}", buf[2 * f]);
        }
        let v = m.voice(id).expect("same voice id keeps playing");
        assert_eq!((v.snd, v.looped), (1, true));
    }

    #[test]
    fn positional_gain_and_pan_follow_the_listener() {
        let f = Falloff { reference: 100.0, max: 500.0, factor: 1.0 };
        assert_eq!(f.gain(50.0), 1.0);
        assert!((f.gain(300.0) - 0.5).abs() < 1e-9);
        assert_eq!(f.gain(900.0), 0.0);
        let mut m = mixer_with(vec![0.5; 2000], SoundMeta::default(), 0);
        m.listener = (0.0, 0.0);
        m.play(0, PlayOpts { spatial: Spatial::At { x: 300.0, y: 0.0, falloff: f }, ..Default::default() });
        let mut buf = vec![0.0; 100];
        m.render(&mut buf);
        // gain 0.5, pan 300/500 = 0.6 to the right: left 0.4, right 1
        assert!((buf[0] - 0.5 * 0.5 * 0.4).abs() < 1e-5, "{}", buf[0]);
        assert!((buf[1] - 0.25).abs() < 1e-5, "{}", buf[1]);
        // emitters: moving it changes the mix; freeing it stops its voices
        let mut m = mixer_with(vec![0.5; 20000], SoundMeta::default(), 0);
        m.emitters.push(Some(Emitter { x: -50.0, y: 0.0, gain: 1.0, pitch: 1.0, falloff: f }));
        m.play(0, PlayOpts { spatial: Spatial::Emitter(0), ..Default::default() });
        let mut buf = vec![0.0; 100];
        m.render(&mut buf);
        assert!(buf[0] > buf[1], "left of the listener");
        m.emitters[0] = None;
        m.advance(100);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn voice_limit_steals_the_lowest_priority() {
        let mut m = mixer_with(vec![0.0; 2000], SoundMeta::default(), 0);
        let first = m.play(0, PlayOpts { priority: 0.0, ..Default::default() });
        for _ in 1..MAX_VOICES {
            m.play(0, PlayOpts { priority: 5.0, ..Default::default() });
        }
        let id = m.play(0, PlayOpts { priority: 1.0, ..Default::default() });
        assert!(m.voice(first).is_none() && m.voice(id).is_some());
        assert_eq!(m.play(0, PlayOpts { priority: 0.0, ..Default::default() }), 0, "too low to steal");
    }

    #[test]
    fn decodes_wav_and_resamples_to_44100() {
        // 22050 Hz mono 16-bit WAV, 0.1 s of a 1 kHz square
        let n = 2205u32;
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + n * 2).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(22050u32.to_le_bytes());
        w.extend((22050u32 * 2).to_le_bytes());
        w.extend(2u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"data");
        w.extend((n * 2).to_le_bytes());
        for i in 0..n {
            let v: i16 = if (i / 11) % 2 == 0 { 8000 } else { -8000 };
            w.extend(v.to_le_bytes());
        }
        let pcm = decode(&w).unwrap();
        assert_eq!(pcm.len() / 2, 4410);
        assert!(pcm[0] > 0.2 && pcm[0] == pcm[1]);
    }
}
