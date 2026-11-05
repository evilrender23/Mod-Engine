//! Object editor: properties, events (own and inherited), methods and variables.

use crate::app::{Cx, Open};
use crate::highlight::{code_editor, compile_error};
use crate::objsrc::{splice, write_object, Item, ItemKind, ObjectSrc};
use eframe::egui::{self, RichText, Sense, Vec2};
use modengine::input::KEY_NAMES;

#[derive(Clone, PartialEq, Debug)]
enum Sel {
    Event(String, Option<String>),
    Inherited(String, Option<String>, String),
    Method(String),
    Vars,
}

pub struct ObjectEditor {
    pub name: String,
    sel: Option<Sel>,
    checked: Option<u64>,
    error: Option<(u32, String)>,
    new_name: String,
}

/// Properties with their own widgets; everything else shows up under "Variables".
const KNOWN: &[&str] = &["sprite", "mask", "depth", "visible", "solid", "persistent"];

const SIMPLE_EVENTS: &[(&str, &str)] = &[
    ("create", "al crearse"),
    ("destroy", "al destruirse"),
    ("step_begin", "inicio de cada paso"),
    ("step", "cada paso"),
    ("step_end", "final de cada paso"),
    ("draw_begin", "antes de dibujar"),
    ("draw", "dibujar (sustituye al sprite)"),
    ("draw_end", "después de dibujar"),
    ("draw_gui", "dibujar encima de todo, en pantalla"),
    ("room_start", "al entrar en la room"),
    ("room_end", "al salir de la room"),
    ("game_start", "al empezar el juego"),
    ("game_end", "al cerrar el juego"),
    ("outside_room", "sale de la room"),
    ("boundary", "toca el borde de la room"),
    ("animation_end", "la animación termina"),
    ("mouse_enter", "el ratón entra"),
    ("mouse_leave", "el ratón sale"),
];

fn key_arg(k: &str) -> String {
    if k.chars().all(|c| c.is_ascii_digit()) {
        format!("key.n{k}")
    } else {
        format!("key.{k}")
    }
}

fn clean(v: &str) -> &str {
    v.split("//").next().unwrap_or("").trim()
}

/// Turns a move inside a filtered list (only events, or only methods) into a move of the
/// object's items: `rows[k]` is the item index of the k-th row; `to` is an insertion index.
/// The item lands right before the row it was dropped on (or right after the last row).
fn item_move(rows: &[usize], from: usize, to: usize) -> (usize, usize) {
    let target = match rows.get(to) {
        Some(i) => *i,
        None => rows.last().map(|i| i + 1).unwrap_or(0),
    };
    (rows[from], target)
}

fn event_label(name: &str, arg: &Option<String>) -> String {
    match arg {
        Some(a) => format!("{name}({a})"),
        None => name.to_string(),
    }
}

impl ObjectEditor {
    pub fn new(name: &str) -> Self {
        ObjectEditor { name: name.to_string(), sel: None, checked: None, error: None, new_name: String::new() }
    }

    pub fn script_rel(&self, p: &crate::project::Project) -> Option<String> {
        p.object(&self.name).map(|o| p.scripts[o.script].rel.clone())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let Some(entry) = cx.project.object(&self.name) else {
            ui.label(format!("El objeto '{}' ya no existe.", self.name));
            return;
        };
        let si = entry.script;
        let rel = cx.project.scripts[si].rel.clone();
        let Some(original) = entry.model.clone() else {
            ui.label("Este objeto usa construcciones que el editor visual no entiende; edítalo como texto.");
            if ui.button(format!("Abrir {rel}")).clicked() {
                cx.open.push(Open::Script(rel));
            }
            return;
        };
        let rev = cx.project.scripts[si].rev;
        if self.checked != Some(rev) {
            self.error = compile_error(&rel, &cx.project.scripts[si].text);
            self.checked = Some(rev);
        }
        let mut m = original.clone();
        if self.sel.is_none() {
            self.sel = m.items.iter().find_map(|i| match &i.kind {
                ItemKind::Event { name, arg, .. } => Some(Sel::Event(name.clone(), arg.clone())),
                _ => None,
            });
            if self.sel.is_none() {
                self.sel = Some(Sel::Vars);
            }
        }

        egui::Panel::left("object_side").resizable(true).default_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, cx, &mut m, &rel));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some((line, msg)) = &self.error {
                ui.colored_label(crate::theme::colors(ui.ctx()).error, format!("{rel}:{line}: {msg}"));
            }
            self.main(ui, cx, &mut m);
        });

        if m != original {
            let text = splice(&cx.project.scripts[si].text, &self.name, &write_object(&m));
            cx.project.set_script_text(si, text);
        }
    }

    fn side(&mut self, ui: &mut egui::Ui, cx: &mut Cx, m: &mut ObjectSrc, rel: &str) {
        ui.horizontal(|ui| {
            ui.heading(&self.name);
            if ui.small_button(rel).on_hover_text("ver el fichero completo").clicked() {
                cx.open.push(Open::Script(rel.to_string()));
            }
        });
        ui.add_space(4.0);

        // ---- sprite & mask
        let sprites = cx.project.sprite_names();
        let sprite = m.prop("sprite").map(|v| clean(v).to_string());
        let sprite_row = ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
            crate::tex::checkerboard(ui.painter(), rect, 8.0);
            let shown = sprite.clone().or_else(|| cx.project.object_sprite(&self.name));
            if let Some(s) = shown {
                if let Some(t) = cx.tex.get(ui.ctx(), cx.project, &s) {
                    t.draw_fit(ui.painter(), 0, rect.shrink(2.0));
                }
            }
            ui.vertical(|ui| {
                for (prop, label) in [("sprite", "Sprite"), ("mask", "Máscara")] {
                    let cur = m.prop(prop).map(|v| clean(v).to_string());
                    let mut sel = cur.clone();
                    ui.horizontal(|ui| {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(("obj", prop)).selected_text(sel.clone().unwrap_or_else(|| "(ninguno)".into())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut sel, None, "(ninguno)");
                            for s in &sprites {
                                ui.selectable_value(&mut sel, Some(s.clone()), s);
                            }
                        });
                        if let Some(s) = &sel {
                            if ui.small_button("✎").on_hover_text("editar sprite").clicked() {
                                cx.open.push(Open::Sprite(s.clone()));
                            }
                        }
                    });
                    if sel != cur {
                        m.set_prop(prop, sel);
                    }
                }
            });
        });
        // a sprite dragged from the asset browser becomes the object's sprite
        if let Some(Open::Sprite(_)) = sprite_row.response.dnd_hover_payload::<Open>().as_deref() {
            crate::dnd::drop_hint(ui, sprite_row.response.rect, "");
        }
        if let Some(Open::Sprite(s)) = sprite_row.response.dnd_release_payload::<Open>().as_deref() {
            m.set_prop("sprite", Some(s.clone()));
        }

        // ---- parent
        let names = cx.project.object_names();
        let descendants: Vec<String> = names.iter().filter(|n| is_descendant(cx.project, n, &self.name)).cloned().collect();
        let mut parent = m.parent.clone();
        let parent_row = ui.horizontal(|ui| {
            ui.label("Padre");
            egui::ComboBox::from_id_salt("parent").selected_text(parent.clone().unwrap_or_else(|| "(ninguno)".into())).show_ui(ui, |ui| {
                ui.selectable_value(&mut parent, None, "(ninguno)");
                for n in &names {
                    if *n != self.name && !descendants.contains(n) {
                        ui.selectable_value(&mut parent, Some(n.clone()), n);
                    }
                }
            });
            if let Some(p) = &parent {
                if ui.small_button("→").on_hover_text("abrir el padre").clicked() {
                    cx.open.push(Open::Object(p.clone()));
                }
            }
        });
        // an object dragged from the asset browser becomes the parent (no cycles)
        let can_parent = |o: &Open| matches!(o, Open::Object(n) if *n != self.name && !descendants.contains(n));
        if parent_row.response.dnd_hover_payload::<Open>().is_some_and(|o| can_parent(&o)) {
            crate::dnd::drop_hint(ui, parent_row.response.rect, "");
        }
        if let Some(o) = parent_row.response.dnd_release_payload::<Open>() {
            if let (true, Open::Object(n)) = (can_parent(&o), &*o) {
                parent = Some(n.clone());
            }
        }
        if parent != m.parent {
            m.parent = parent;
        }
        let children: Vec<String> = cx.project.objects.iter().filter(|o| o.parent.as_deref() == Some(&self.name)).map(|o| o.name.clone()).collect();
        if !children.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Hijos:").weak());
                for c in children {
                    if ui.link(&c).clicked() {
                        cx.open.push(Open::Object(c));
                    }
                }
            });
        }

        // ---- flags
        ui.horizontal(|ui| {
            for (prop, label, default) in [("visible", "visible", true), ("solid", "sólido", false), ("persistent", "persistente", false)] {
                let cur = m.prop(prop).map(|v| clean(v) == "true").unwrap_or(default);
                let mut v = cur;
                if ui.checkbox(&mut v, label).changed() {
                    m.set_prop(prop, if v == default { None } else { Some(v.to_string()) });
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Profundidad");
            let cur = m.prop("depth").and_then(|v| clean(v).parse::<f64>().ok());
            let mut d = cur.unwrap_or(0.0);
            if ui.add(egui::DragValue::new(&mut d).speed(1.0)).changed() {
                m.set_prop("depth", if d == 0.0 { None } else { Some(modscript::fmt_num(d)) });
            }
            ui.label(RichText::new("(mayor = más al fondo)").weak());
        });

        ui.separator();

        // ---- events
        ui.horizontal(|ui| {
            ui.strong("Eventos");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some((name, arg)) = add_event_menu(ui, &names) {
                    if m.event_index(&name, arg.as_deref()).is_none() {
                        m.items.push(Item { comments: vec![], kind: ItemKind::Event { name: name.clone(), arg: arg.clone(), body: String::new() } });
                    }
                    self.sel = Some(Sel::Event(name, arg));
                }
            });
        });
        let mut delete: Option<usize> = None;
        // events and methods can be dragged to change their order in the file
        let mut moves: Vec<(usize, usize)> = vec![];
        let ev_list = ui.scope_id().with("object_events");
        let (mut ev_rows, mut ev_rects) = (vec![], vec![]);
        for (i, item) in m.items.iter().enumerate() {
            if let ItemKind::Event { name, arg, .. } = &item.kind {
                let s = Sel::Event(name.clone(), arg.clone());
                let label = format!("⚡ {}", event_label(name, arg));
                let dragged = crate::dnd::dragging_from(ui.ctx(), ev_list) == Some(ev_rows.len());
                let r = ui.add(egui::Button::selectable(self.sel.as_ref() == Some(&s) || dragged, label.clone()).sense(Sense::click_and_drag()));
                crate::dnd::source(&r, || crate::dnd::ListDrag { list: ev_list, from: ev_rows.len(), label });
                ev_rows.push(i);
                ev_rects.push(r.rect);
                if r.clicked() {
                    self.sel = Some(s);
                }
                r.context_menu(|ui| {
                    if ui.button("Eliminar evento").clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            }
        }
        if let Some((from, to)) = crate::dnd::reorder(ui, ev_list, &ev_rects, true) {
            moves.push(item_move(&ev_rows, from, to));
        }
        for (owner, name, arg, _) in inherited_events(cx.project, m) {
            let s = Sel::Inherited(name.clone(), arg.clone(), owner.clone());
            let label = RichText::new(format!("↳ {}   de {owner}", event_label(&name, &arg))).italics().color(crate::theme::colors(ui.ctx()).weak);
            if ui.add(egui::Button::selectable(self.sel.as_ref() == Some(&s), label)).clicked() {
                self.sel = Some(s);
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Métodos");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = crate::tomlw::valid_name(self.new_name.trim());
                if ui.add_enabled(ok, egui::Button::new("+")).on_hover_text("añadir método").clicked() {
                    let n = self.new_name.trim().to_string();
                    m.items.push(Item { comments: vec![], kind: ItemKind::Method { name: n.clone(), params: String::new(), body: String::new() } });
                    self.sel = Some(Sel::Method(n));
                    self.new_name.clear();
                }
                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("nombre").desired_width(110.0));
            });
        });
        let fn_list = ui.scope_id().with("object_methods");
        let (mut fn_rows, mut fn_rects) = (vec![], vec![]);
        for (i, item) in m.items.iter().enumerate() {
            if let ItemKind::Method { name, params, .. } = &item.kind {
                let s = Sel::Method(name.clone());
                let label = format!("ƒ {name}({params})");
                let dragged = crate::dnd::dragging_from(ui.ctx(), fn_list) == Some(fn_rows.len());
                let r = ui.add(egui::Button::selectable(self.sel.as_ref() == Some(&s) || dragged, label.clone()).sense(Sense::click_and_drag()));
                crate::dnd::source(&r, || crate::dnd::ListDrag { list: fn_list, from: fn_rows.len(), label });
                fn_rows.push(i);
                fn_rects.push(r.rect);
                if r.clicked() {
                    self.sel = Some(s);
                }
                r.context_menu(|ui| {
                    if ui.button("Eliminar método").clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            }
        }
        if let Some((from, to)) = crate::dnd::reorder(ui, fn_list, &fn_rects, true) {
            moves.push(item_move(&fn_rows, from, to));
        }
        for (from, to) in moves {
            crate::dnd::move_item(&mut m.items, from, to);
        }
        ui.separator();
        let vars = m.items.iter().filter(|i| matches!(&i.kind, ItemKind::Prop { name, .. } if !KNOWN.contains(&name.as_str()))).count();
        if ui.add(egui::Button::selectable(self.sel == Some(Sel::Vars), format!("𝑥 Variables ({vars})"))).clicked() {
            self.sel = Some(Sel::Vars);
        }
        if let Some(i) = delete {
            m.items.remove(i);
            self.sel = None;
        }
    }

    pub fn show_vars(&mut self) {
        self.sel = Some(Sel::Vars);
    }

    fn main(&mut self, ui: &mut egui::Ui, cx: &mut Cx, m: &mut ObjectSrc) {
        match self.sel.clone() {
            Some(Sel::Event(name, arg)) => {
                let Some(i) = m.event_index(&name, arg.as_deref()) else {
                    self.sel = None;
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("on {}", event_label(&name, &arg))).monospace().strong().size(16.0));
                    if let Some((_, d)) = SIMPLE_EVENTS.iter().find(|(n, _)| *n == name) {
                        ui.label(RichText::new(*d).weak());
                    }
                    if let Some(parent_has) = inherited_events(cx.project, &ObjectSrc { items: vec![], ..m.clone() }).into_iter().find(|(_, n, a, _)| *n == name && *a == arg) {
                        ui.label(RichText::new(format!("(sustituye al de {}; usa inherited() para ejecutarlo también)", parent_has.0)).weak());
                    }
                });
                if let ItemKind::Event { body, .. } = &mut m.items[i].kind {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        code_editor(ui, body, ("event", &self.name, &name, &arg), 30);
                    });
                }
            }
            Some(Sel::Inherited(name, arg, owner)) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("on {}", event_label(&name, &arg))).monospace().strong().size(16.0));
                    ui.label(RichText::new(format!("heredado de {owner}")).weak());
                    if ui.button("Sobrescribir aquí").clicked() {
                        m.items.push(Item { comments: vec![], kind: ItemKind::Event { name: name.clone(), arg: arg.clone(), body: "inherited()\n".into() } });
                        self.sel = Some(Sel::Event(name.clone(), arg.clone()));
                    }
                    if ui.button(format!("Abrir {owner}")).clicked() {
                        cx.open.push(Open::Object(owner.clone()));
                    }
                });
                let body = inherited_events(cx.project, m).into_iter().find(|(o, n, a, _)| *o == owner && *n == name && *a == arg).map(|x| x.3).unwrap_or_default();
                let mut ro = body;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_enabled_ui(false, |ui| code_editor(ui, &mut ro, ("inh", &name, &arg), 20));
                });
            }
            Some(Sel::Method(name)) => {
                let Some(i) = m.items.iter().position(|it| matches!(&it.kind, ItemKind::Method { name: n, .. } if *n == name)) else {
                    self.sel = None;
                    return;
                };
                if let ItemKind::Method { params, body, .. } = &mut m.items[i].kind {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("function {name}(")).monospace().strong().size(16.0));
                        ui.add(egui::TextEdit::singleline(params).font(egui::TextStyle::Monospace).desired_width(260.0).hint_text("a, b = 2"));
                        ui.label(RichText::new(")").monospace().strong().size(16.0));
                    });
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        code_editor(ui, body, ("method", &self.name, &name), 30);
                    });
                }
            }
            Some(Sel::Vars) | None => self.vars(ui, cx, m),
        }
    }

    fn vars(&mut self, ui: &mut egui::Ui, cx: &mut Cx, m: &mut ObjectSrc) {
        use crate::typedvar as tv;
        ui.label(RichText::new("Variables de instancia").strong().size(16.0));
        ui.label(RichText::new("Cada instancia nueva recibe estos valores antes de su evento create (los del padre primero). Con un tipo, el editor de rooms muestra el control adecuado y el motor comprueba los valores.").weak());
        ui.add_space(6.0);
        let inherited = tv::inherited_vars(cx.project, m.parent.as_deref());
        let project: &crate::project::Project = cx.project;
        let mut remove = None;
        let mut retype: Option<(usize, Option<modscript::VarType>)> = None;
        egui::Grid::new("vars").num_columns(4).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
            ui.label(RichText::new("nombre").weak());
            ui.label(RichText::new("tipo").weak());
            ui.label(RichText::new("valor inicial").weak());
            ui.label("");
            ui.end_row();
            for (i, item) in m.items.iter_mut().enumerate() {
                let ItemKind::Prop { name, ty, value } = &mut item.kind else { continue };
                if KNOWN.contains(&name.as_str()) {
                    continue;
                }
                let parent_decl = inherited.iter().find(|v| v.name == *name);
                ui.add(egui::TextEdit::singleline(name).font(egui::TextStyle::Monospace).desired_width(130.0).clip_text(false));
                // type column: own type, inherited type, or untyped
                let own = ty.as_deref().and_then(|t| modscript::parse_var_type(t).ok());
                let eff = own.clone().or_else(|| parent_decl.map(|v| v.ty.clone()));
                ui.horizontal(|ui| {
                    if let (None, Some(pd)) = (&own, parent_decl) {
                        ui.label(RichText::new(pd.ty.to_source()).monospace().weak()).on_hover_text(format!("tipo declarado en {}", pd.owner));
                    } else if ty.is_some() && own.is_none() {
                        ui.add(egui::TextEdit::singleline(ty.as_mut().unwrap()).font(egui::TextStyle::Monospace).desired_width(110.0));
                        ui.colored_label(crate::theme::colors(ui.ctx()).error, "tipo no válido");
                    } else if let Some(t) = tv::type_editor(ui, ("ty", i), own.as_ref()) {
                        retype = Some((i, t));
                    }
                });
                // value column
                ui.horizontal(|ui| {
                    let (code, comment) = tv::split_comment(value);
                    let (code, comment) = (code.to_string(), comment.map(str::to_string));
                    let code = code.as_str();
                    match eff.as_ref().and_then(|t| tv::from_source(t, code).map(|p| (t.clone(), p))) {
                        Some((t, mut plain)) => {
                            if tv::widget(ui, ("val", i), &t, &mut plain, project) {
                                *value = match comment {
                                    Some(c) => format!("{}      {c}", tv::to_source(&plain)),
                                    None => tv::to_source(&plain),
                                };
                            }
                            if let Err(e) = check_plain(&t, &plain) {
                                ui.colored_label(crate::theme::colors(ui.ctx()).error, e);
                            }
                        }
                        None => {
                            ui.add(egui::TextEdit::singleline(value).font(egui::TextStyle::Monospace).desired_width(260.0));
                            if eff.is_some() {
                                ui.label(RichText::new("expresión").weak()).on_hover_text("No es un valor literal: el motor lo comprobará al crear cada instancia.");
                            }
                        }
                    }
                });
                if ui.small_button("🗑").on_hover_text(if parent_decl.is_some() { "quitar la sobrescritura" } else { "eliminar variable" }).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some((i, t)) = retype {
            if let ItemKind::Prop { ty, value, .. } = &mut m.items[i].kind {
                let keep = t.as_ref().is_some_and(|t| tv::from_source(t, tv::split_comment(value).0).is_some_and(|p| check_plain(t, &p).is_ok()));
                if let (Some(t), false) = (&t, keep) {
                    *value = tv::to_source(&tv::default_for(t, cx.project));
                }
                *ty = t.map(|t| t.to_source());
            }
        }
        if let Some(i) = remove {
            m.items.remove(i);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("nueva variable").desired_width(140.0));
            let n = self.new_name.trim().to_string();
            let builtin = modengine::instance::BUILTIN_VARS.iter().any(|(b, _)| *b == n);
            let ok = crate::tomlw::valid_name(&n) && m.prop(&n).is_none() && !KNOWN.contains(&n.as_str());
            if ui.add_enabled(ok && !builtin, egui::Button::new("Añadir con tipo")).clicked() {
                m.set_prop(&n, Some("0".into()));
                if let Some(ItemKind::Prop { ty, .. }) = m.items.iter_mut().map(|i| &mut i.kind).find(|k| matches!(k, ItemKind::Prop { name, .. } if *name == n)) {
                    *ty = Some("num".into());
                }
                self.new_name.clear();
            }
            if ui.add_enabled(ok, egui::Button::new("Añadir sin tipo")).clicked() {
                m.set_prop(&n, Some("0".into()));
                self.new_name.clear();
            }
            if builtin {
                ui.label(RichText::new("variable integrada: no admite tipo").weak());
            }
        });

        // inherited typed variables not overridden here
        let own: Vec<String> = m.items.iter().filter_map(|i| if let ItemKind::Prop { name, .. } = &i.kind { Some(name.clone()) } else { None }).collect();
        let pending: Vec<_> = inherited.iter().filter(|v| !own.contains(&v.name)).collect();
        if !pending.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Heredadas").strong());
            let weak = crate::theme::colors(ui.ctx()).weak;
            let mut add = None;
            egui::Grid::new("inh_vars").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                for v in pending {
                    ui.label(RichText::new(&v.name).monospace().color(weak));
                    ui.label(RichText::new(v.ty.to_source()).monospace().color(weak));
                    ui.label(RichText::new(format!("{}   (de {})", v.value, v.set_by)).monospace().color(weak));
                    if ui.small_button("Sobrescribir").clicked() {
                        add = Some((v.name.clone(), v.value.clone()));
                    }
                    ui.end_row();
                }
            });
            if let Some((n, val)) = add {
                m.set_prop(&n, Some(val));
            }
        }
    }
}

/// Checks a literal against its type (asset existence is checked by the engine on save).
fn check_plain(t: &modscript::VarType, p: &crate::typedvar::Plain) -> Result<(), String> {
    use crate::typedvar::Plain;
    use modscript::Value;
    let v = match p {
        Plain::Num(n) => Value::Num(*n),
        Plain::Bool(b) => Value::Bool(*b),
        Plain::Str(s) => Value::str(s),
        Plain::Color(c) => Value::Num(((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32) as f64),
        Plain::Asset(_) => return Ok(()),
    };
    t.check(&v)
}

fn is_descendant(p: &crate::project::Project, name: &str, ancestor: &str) -> bool {
    let mut cur = p.object(name).and_then(|o| o.parent.clone());
    let mut guard = 0;
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        guard += 1;
        if guard > 32 {
            return false;
        }
        cur = p.object(&c).and_then(|o| o.parent.clone());
    }
    false
}

/// Events defined by ancestors and not overridden by `m`: (owner, name, arg, body).
fn inherited_events(p: &crate::project::Project, m: &ObjectSrc) -> Vec<(String, String, Option<String>, String)> {
    let mut out: Vec<(String, String, Option<String>, String)> = Vec::new();
    let mut cur = m.parent.clone();
    let mut guard = 0;
    while let Some(c) = cur {
        guard += 1;
        if guard > 32 {
            break;
        }
        let Some(o) = p.object(&c) else { break };
        if let Some(model) = &o.model {
            for it in &model.items {
                if let ItemKind::Event { name, arg, body } = &it.kind {
                    let shadowed = m.event_index(name, arg.as_deref()).is_some() || out.iter().any(|(_, n, a, _)| n == name && a == arg);
                    if !shadowed {
                        out.push((c.clone(), name.clone(), arg.clone(), body.clone()));
                    }
                }
            }
        }
        cur = o.parent.clone();
    }
    out
}

fn add_event_menu(ui: &mut egui::Ui, objects: &[String]) -> Option<(String, Option<String>)> {
    let mut picked = None;
    ui.menu_button("+ Evento", |ui| {
        for (n, d) in SIMPLE_EVENTS {
            if ui.button(*n).on_hover_text(*d).clicked() {
                picked = Some((n.to_string(), None));
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button("collision(…)", |ui| {
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for o in objects {
                    if ui.button(o).clicked() {
                        picked = Some(("collision".into(), Some(o.clone())));
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button("alarm(…)", |ui| {
            for n in 0..12 {
                if ui.button(format!("alarm({n})")).clicked() {
                    picked = Some(("alarm".into(), Some(n.to_string())));
                    ui.close();
                }
            }
        });
        for (ev, label) in [("key", "tecla mantenida"), ("key_pressed", "tecla pulsada"), ("key_released", "tecla soltada")] {
            ui.menu_button(format!("{ev}(…)"), |ui| {
                ui.label(RichText::new(label).weak());
                if ui.button("cualquier tecla").clicked() {
                    picked = Some((ev.into(), None));
                    ui.close();
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for k in KEY_NAMES {
                        if ui.button(*k).clicked() {
                            picked = Some((ev.into(), Some(key_arg(k))));
                            ui.close();
                        }
                    }
                });
            });
        }
        ui.menu_button("ratón", |ui| {
            for ev in ["mouse_pressed", "mouse_released", "mouse_down", "global_mouse_pressed", "global_mouse_released", "global_mouse_down"] {
                ui.menu_button(ev, |ui| {
                    for b in ["left", "right", "middle"] {
                        if ui.button(b).clicked() {
                            picked = Some((ev.into(), Some(format!("mouse.{b}"))));
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.menu_button("user(…)", |ui| {
            for n in 0..16 {
                if ui.button(format!("user({n})")).clicked() {
                    picked = Some(("user".into(), Some(n.to_string())));
                    ui.close();
                }
            }
        });
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LogLine;
    use crate::project::Project;
    use crate::tex::TexCache;
    use egui_kittest::{kittest::Queryable, Harness};

    struct St {
        ed: ObjectEditor,
        project: Project,
        tex: TexCache,
        log: Vec<LogLine>,
        open: Vec<Open>,
    }

    #[test]
    fn add_event_from_menu_and_toggle_solid() {
        let dir = crate::room::tests::temp_project("object");
        let project = Project::open(&dir).unwrap();
        let st = St { ed: ObjectEditor::new("Spike"), project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 800.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run();
        h.get_by_label("+ Evento").click();
        h.run();
        h.get_by_label("step").click();
        h.run();
        h.get_by_label("sólido").click();
        h.run();
        let p = &h.state().project;
        let o = p.object("Spike").unwrap();
        let text = &p.scripts[o.script].text;
        assert!(text.contains("object Spike {\n    sprite = spike\n    solid = true\n\n    on step {\n    }\n}"), "{text}");
        assert!(p.scripts[o.script].dirty());
        modscript::parse_file("x.ms", text).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn item_move_maps_list_rows_to_items() {
        // items: 0 prop, 1 event, 2 method, 3 event, 4 event
        let events = [1, 3, 4];
        assert_eq!(item_move(&events, 2, 0), (4, 1)); // last event before the first one
        assert_eq!(item_move(&events, 0, 3), (1, 5)); // first event after the last one
        assert_eq!(item_move(&events, 0, 2), (1, 4));
    }

    #[test]
    fn drag_an_event_above_another() {
        let dir = crate::room::tests::temp_project("object_dnd");
        let project = Project::open(&dir).unwrap();
        let st = St { ed: ObjectEditor::new("Player"), project, tex: TexCache::default(), log: vec![], open: vec![] };
        let mut h = Harness::builder().with_size([1200.0, 800.0]).allow_missing_glyphs().with_accessibility_check(false).build_ui_state(
            |ui, s: &mut St| {
                let mut cx = Cx { project: &mut s.project, tex: &mut s.tex, log: &mut s.log, open: &mut s.open, active: true };
                s.ed.ui(ui, &mut cx);
            },
            st,
        );
        h.run_steps(3);
        let from = h.get_by_label("⚡ alarm(0)").rect().center();
        let step = h.get_by_label("⚡ step").rect();
        crate::room::tests::drag(&mut h, from, step.center() - Vec2::new(0.0, 5.0));
        let p = &h.state().project;
        let o = p.object("Player").unwrap();
        let text = &p.scripts[o.script].text;
        let (alarm, step) = (text.find("on alarm(0)").unwrap(), text.find("on step").unwrap());
        assert!(alarm < step, "alarm(0) now comes first:\n{text}");
        assert!(text.contains("function die()") && text.contains("on collision(Spike)"));
        modscript::parse_file("x.ms", text).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
