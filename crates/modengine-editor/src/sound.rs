//! Sound editor: synthesized effects (with live preview) or audio files.

use crate::app::Cx;
use crate::tomlw::{num, q};
use eframe::egui::{self, RichText};
use modengine::synth::{render_wav, SynthDef};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default)]
#[serde(default)]
struct SoundToml {
    file: Option<String>,
    synth: Option<SynthDef>,
    volume: Option<f64>,
}

pub struct SoundEditor {
    pub name: String,
    path: PathBuf,
    is_toml: bool,
    file: Option<String>,
    synth: Option<SynthDef>,
    volume: f64,
    pub dirty: bool,
}

/// Plays a file with whatever command-line player the system has.
pub fn play_file(path: &Path) -> Result<(), String> {
    for player in ["pw-play", "paplay", "aplay", "afplay"] {
        if std::process::Command::new(player).arg(path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("no se encontró ningún reproductor (pw-play, paplay, aplay)".into())
}

impl SoundEditor {
    pub fn open(name: &str, path: &Path, is_toml: bool) -> Result<Self, String> {
        let t: SoundToml = if is_toml {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            SoundToml { file: path.file_name().map(|f| f.to_string_lossy().to_string()), ..Default::default() }
        };
        Ok(SoundEditor { name: name.into(), path: path.into(), is_toml, file: t.file, synth: t.synth, volume: t.volume.unwrap_or(1.0), dirty: false })
    }

    fn toml_text(&self) -> String {
        let mut s = String::new();
        if let Some(f) = &self.file {
            s += &format!("file = {}\n", q(f));
        }
        if self.volume != 1.0 {
            s += &format!("volume = {}\n", num(self.volume));
        }
        if let Some(d) = &self.synth {
            s += "\n[synth]\n";
            s += &format!("wave = {}\nfreq = {}\n", q(&d.wave), num(d.freq));
            if let Some(e) = d.freq_end {
                s += &format!("freq_end = {}\n", num(e));
            }
            s += &format!("duration = {}\nvolume = {}\nattack = {}\nrelease = {}\n", num(d.duration), num(d.volume), num(d.attack), num(d.release));
            if d.wave == "square" {
                s += &format!("duty = {}\n", num(d.duty));
            }
            if d.vibrato > 0.0 {
                s += &format!("vibrato = {}\nvibrato_speed = {}\n", num(d.vibrato), num(d.vibrato_speed));
            }
            if d.repeat > 1 {
                s += &format!("repeat = {}\nrepeat_pitch = {}\ngap = {}\n", d.repeat, num(d.repeat_pitch), num(d.gap));
            }
        }
        s
    }

    pub fn save(&mut self, cx: &mut Cx) -> Result<(), String> {
        let path = if self.is_toml { self.path.clone() } else { self.path.with_extension("toml") };
        std::fs::write(&path, self.toml_text()).map_err(|e| format!("{}: {e}", path.display()))?;
        self.path = path;
        self.is_toml = true;
        self.dirty = false;
        cx.project.reload_lists();
        Ok(())
    }

    fn preview(&self, cx: &mut Cx) {
        let result = match (&self.synth, &self.file) {
            (Some(d), _) => {
                let tmp = std::env::temp_dir().join(format!("modeditor_{}.wav", self.name));
                std::fs::write(&tmp, render_wav(d)).map_err(|e| e.to_string()).and_then(|_| play_file(&tmp))
            }
            (None, Some(f)) => play_file(&self.path.with_file_name(f)),
            _ => Ok(()),
        };
        if let Err(e) = result {
            cx.error(e);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let before = self.toml_text();
        ui.horizontal(|ui| {
            ui.heading(&self.name);
            if ui.button("▶ Escuchar").clicked() {
                self.preview(cx);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Volumen general");
            ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0));
        });
        ui.separator();
        match &mut self.synth {
            None => {
                ui.label(format!("Fichero de audio: {}", self.file.clone().unwrap_or_default()));
                if ui.button("Convertir en efecto sintetizado").clicked() {
                    self.synth = Some(SynthDef::default());
                    self.file = None;
                }
            }
            Some(d) => {
                ui.label(RichText::new("Efecto sintetizado").strong());
                egui::Grid::new("synth").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Onda");
                    ui.horizontal(|ui| {
                        for w in ["square", "saw", "triangle", "sine", "noise"] {
                            ui.selectable_value(&mut d.wave, w.to_string(), w);
                        }
                    });
                    ui.end_row();
                    ui.label("Frecuencia (Hz)");
                    ui.add(egui::Slider::new(&mut d.freq, 20.0..=3000.0).logarithmic(true));
                    ui.end_row();
                    ui.label("Frecuencia final");
                    ui.horizontal(|ui| {
                        let mut slide = d.freq_end.is_some();
                        ui.checkbox(&mut slide, "deslizar");
                        if slide {
                            let mut e = d.freq_end.unwrap_or(d.freq);
                            ui.add(egui::Slider::new(&mut e, 20.0..=3000.0).logarithmic(true));
                            d.freq_end = Some(e);
                        } else {
                            d.freq_end = None;
                        }
                    });
                    ui.end_row();
                    ui.label("Duración (s)");
                    ui.add(egui::Slider::new(&mut d.duration, 0.01..=2.0));
                    ui.end_row();
                    ui.label("Volumen");
                    ui.add(egui::Slider::new(&mut d.volume, 0.0..=1.0));
                    ui.end_row();
                    ui.label("Ataque (s)");
                    ui.add(egui::Slider::new(&mut d.attack, 0.0..=0.5));
                    ui.end_row();
                    ui.label("Caída (s)");
                    ui.add(egui::Slider::new(&mut d.release, 0.0..=1.0));
                    ui.end_row();
                    if d.wave == "square" {
                        ui.label("Ancho de pulso");
                        ui.add(egui::Slider::new(&mut d.duty, 0.05..=0.95));
                        ui.end_row();
                    }
                    ui.label("Vibrato");
                    ui.add(egui::Slider::new(&mut d.vibrato, 0.0..=0.5));
                    ui.end_row();
                    ui.label("Velocidad vibrato");
                    ui.add(egui::Slider::new(&mut d.vibrato_speed, 1.0..=40.0));
                    ui.end_row();
                    ui.label("Repeticiones");
                    ui.add(egui::Slider::new(&mut d.repeat, 1..=8));
                    ui.end_row();
                    ui.label("Tono por repetición");
                    ui.add(egui::Slider::new(&mut d.repeat_pitch, 0.5..=2.0));
                    ui.end_row();
                    ui.label("Silencio entre repeticiones");
                    ui.add(egui::Slider::new(&mut d.gap, 0.0..=0.5));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Ideas:");
                    for (label, wave, f0, f1, dur) in
                        [("salto", "square", 280.0, Some(620.0), 0.12), ("moneda", "square", 990.0, Some(1480.0), 0.1), ("golpe", "noise", 800.0, Some(100.0), 0.2), ("láser", "saw", 1400.0, Some(200.0), 0.18)]
                    {
                        if ui.small_button(label).clicked() {
                            *d = SynthDef { wave: wave.into(), freq: f0, freq_end: f1, duration: dur, ..SynthDef::default() };
                        }
                    }
                });
            }
        }
        if self.toml_text() != before {
            self.dirty = true;
        }
    }
}
