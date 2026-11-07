//! Plain ModScript file editor.

use crate::app::Cx;
use crate::highlight::{code_editor, compile_error};
use eframe::egui::{self, RichText};

pub struct ScriptEditor {
    pub rel: String,
    checked: Option<u64>,
    error: Option<(u32, String)>,
}

impl ScriptEditor {
    pub fn new(rel: &str) -> Self {
        ScriptEditor { rel: rel.to_string(), checked: None, error: None }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let Some(i) = cx.project.script_index(&self.rel) else {
            ui.label("Este fichero ya no existe.");
            return;
        };
        let rev = cx.project.scripts[i].rev;
        if self.checked != Some(rev) {
            self.error = compile_error(&self.rel, &cx.project.scripts[i].text);
            self.checked = Some(rev);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.rel).strong());
            let objs: Vec<String> = cx.project.objects.iter().filter(|o| o.script == i).map(|o| o.name.clone()).collect();
            for o in objs {
                if ui.small_button(format!("{} {o}", fa!(cube))).on_hover_text("abrir en el editor de objetos").clicked() {
                    cx.open.push(crate::app::Open::Object(o));
                }
            }
        });
        match &self.error {
            Some((line, msg)) => ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("línea {line}: {msg}")),
            None => ui.colored_label(crate::theme::colors(ui.ctx()).ok, concat!(fa!(check), " sin errores de sintaxis")),
        };
        let mut text = cx.project.scripts[i].text.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            code_editor(ui, &mut text, ("script", &self.rel), 40);
        });
        cx.project.set_script_text(i, text);
    }
}
