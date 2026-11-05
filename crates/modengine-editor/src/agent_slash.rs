//! Slash commands typed in the Agente message box (`/compactar`, `/modelo opus`…): parsing and
//! the list the autocomplete offers. The panel runs them (`agent_panel.rs`).

use crate::agent::{Kind, Permission};

/// A command and what it takes.
pub struct Spec {
    pub name: &'static str,
    /// Other names it answers to.
    pub aliases: &'static [&'static str],
    /// Shown after the name in the help ("<modelo>"); empty when it takes nothing.
    pub arg: &'static str,
    pub help: &'static str,
}

pub const COMMANDS: &[Spec] = &[
    Spec { name: "nueva", aliases: &["new", "clear"], arg: "", help: "Empieza una conversación nueva (la actual queda en el historial)" },
    Spec { name: "historial", aliases: &["history", "resume"], arg: "[buscar]", help: "Abre el historial de conversaciones del proyecto" },
    Spec { name: "compactar", aliases: &["compact"], arg: "", help: "Resume la conversación para liberar contexto y sigue con el resumen" },
    Spec { name: "modelo", aliases: &["model"], arg: "<modelo|defecto>", help: "Cambia el modelo (opus, sonnet, haiku…; «defecto», el del programa)" },
    Spec { name: "permisos", aliases: &["permissions", "modo"], arg: "<plan|editar|auto>", help: "Qué puede hacer el agente: solo planificar, editar archivos o también ejecutar comandos" },
    Spec { name: "agente", aliases: &["agent"], arg: "<claude|codex|agy>", help: "Cambia de agente (empieza una conversación nueva)" },
    Spec { name: "cancelar", aliases: &["cancel", "detener", "stop", "parar"], arg: "", help: "Cancela el trabajo en curso del agente (Esc); la cola queda en pausa" },
    Spec { name: "reintentar", aliases: &["retry"], arg: "", help: "Vuelve a enviar el último mensaje desde antes de su respuesta" },
    Spec { name: "editar", aliases: &["edit"], arg: "", help: "Edita el último mensaje y lo reenvía (la conversación sigue desde antes de él)" },
    Spec { name: "deshacer", aliases: &["undo", "rewind"], arg: "", help: "Devuelve los archivos que cambió el último turno a como estaban antes" },
    Spec { name: "exportar", aliases: &["export"], arg: "", help: "Guarda la conversación como Markdown (y la copia)" },
    Spec { name: "renombrar", aliases: &["rename"], arg: "<título>", help: "Cambia el título de la conversación" },
    Spec { name: "autocompactar", aliases: &["autocompact"], arg: "<70-95|no>", help: "Compacta sola al pasar ese % del contexto" },
    Spec { name: "coste", aliases: &["cost", "usage", "contexto"], arg: "", help: "Muestra el contexto usado, los tokens y el coste" },
    Spec { name: "ayuda", aliases: &["help", "?"], arg: "", help: "Lista estos comandos" },
];

/// A parsed command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    New,
    History(String),
    Compact,
    /// Empty: the CLI's default.
    Model(Option<String>),
    Permission(Option<Permission>),
    Agent(Option<Kind>),
    Cancel,
    Retry,
    Edit,
    Undo,
    Export,
    Rename(String),
    /// Percent, 0 = off; `None`: show it.
    AutoCompact(Option<u8>),
    Cost,
    Help,
    /// Not a command we know (the name as typed).
    Unknown(String),
}

/// `text` as a command when it starts with `/` (and not `//`, which sends a message that begins with `/`).
pub fn parse(text: &str) -> Option<Command> {
    let t = text.trim();
    let rest = t.strip_prefix('/')?;
    if rest.starts_with('/') || rest.is_empty() {
        return None;
    }
    let (name, arg) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    let name = name.to_lowercase();
    let Some(spec) = find(&name) else { return Some(Command::Unknown(name)) };
    let arg_opt = || (!arg.is_empty()).then(|| arg.to_string());
    Some(match spec.name {
        "nueva" => Command::New,
        "historial" => Command::History(arg.to_string()),
        "compactar" => Command::Compact,
        "modelo" => Command::Model(arg_opt().map(|m| if matches!(m.to_lowercase().as_str(), "defecto" | "default" | "-") { String::new() } else { m })),
        "permisos" => Command::Permission(arg_opt().and_then(|a| permission(&a))),
        "agente" => Command::Agent(arg_opt().and_then(|a| agent(&a))),
        "cancelar" => Command::Cancel,
        "reintentar" => Command::Retry,
        "editar" => Command::Edit,
        "deshacer" => Command::Undo,
        "exportar" => Command::Export,
        "renombrar" => Command::Rename(arg.to_string()),
        "autocompactar" => Command::AutoCompact(arg_opt().and_then(|a| {
            let a = a.trim_end_matches('%').to_lowercase();
            if matches!(a.as_str(), "no" | "off" | "0") {
                Some(0)
            } else {
                a.parse::<u8>().ok().map(|n| n.clamp(50, 98))
            }
        })),
        "coste" => Command::Cost,
        _ => Command::Help,
    })
}

pub fn find(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|c| c.name == name || c.aliases.contains(&name))
}

pub fn permission(a: &str) -> Option<Permission> {
    match a.to_lowercase().as_str() {
        "plan" | "planificar" | "leer" => Some(Permission::Plan),
        "editar" | "edit" | "edits" | "archivos" => Some(Permission::Edit),
        "auto" | "comandos" | "ejecutar" | "todo" => Some(Permission::Auto),
        _ => None,
    }
}

pub fn agent(a: &str) -> Option<Kind> {
    match a.to_lowercase().as_str() {
        "claude" | "claude-code" | "cc" => Some(Kind::Claude),
        "codex" => Some(Kind::Codex),
        "agy" | "antigravity" | "gemini" => Some(Kind::Antigravity),
        _ => None,
    }
}

/// Commands whose name (or alias) starts with what follows the `/` being typed, while the
/// name is still being typed (no space yet).
pub fn suggestions(input: &str) -> Vec<&'static Spec> {
    let Some(q) = input.strip_prefix('/') else { return vec![] };
    if q.starts_with('/') || q.contains(char::is_whitespace) {
        return vec![];
    }
    let q = q.to_lowercase();
    COMMANDS.iter().filter(|c| c.name.starts_with(&q) || c.aliases.iter().any(|a| a.starts_with(&q))).collect()
}

/// The `/ayuda` text.
pub fn help() -> String {
    let mut t = String::from("Comandos (escribe / para verlos; Tab completa):\n");
    for c in COMMANDS {
        let arg = if c.arg.is_empty() { String::new() } else { format!(" {}", c.arg) };
        t += &format!("/{}{arg}: {}\n", c.name, c.help);
    }
    t + "Empieza con // para enviar un mensaje que empiece por /."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse("hola"), None);
        assert_eq!(parse("//ruta/absoluta"), None);
        assert_eq!(parse("/"), None);
        assert_eq!(parse("/nueva"), Some(Command::New));
        assert_eq!(parse("  /compact "), Some(Command::Compact));
        assert_eq!(parse("/modelo opus"), Some(Command::Model(Some("opus".into()))));
        assert_eq!(parse("/model default"), Some(Command::Model(Some(String::new()))));
        assert_eq!(parse("/modelo"), Some(Command::Model(None)));
        assert_eq!(parse("/permisos plan"), Some(Command::Permission(Some(Permission::Plan))));
        assert_eq!(parse("/permisos editar"), Some(Command::Permission(Some(Permission::Edit))));
        assert_eq!(parse("/permisos xyz"), Some(Command::Permission(None)));
        assert_eq!(parse("/agente codex"), Some(Command::Agent(Some(Kind::Codex))));
        assert_eq!(parse("/agente Antigravity"), Some(Command::Agent(Some(Kind::Antigravity))));
        assert_eq!(parse("/historial nivel"), Some(Command::History("nivel".into())));
        assert_eq!(parse("/renombrar Mi juego"), Some(Command::Rename("Mi juego".into())));
        assert_eq!(parse("/autocompactar 85%"), Some(Command::AutoCompact(Some(85))));
        assert_eq!(parse("/autocompactar no"), Some(Command::AutoCompact(Some(0))));
        assert_eq!(parse("/undo"), Some(Command::Undo));
        assert_eq!(parse("/cancelar"), Some(Command::Cancel));
        assert_eq!(parse("/stop"), Some(Command::Cancel));
        assert_eq!(parse("/frobnicar"), Some(Command::Unknown("frobnicar".into())));
        assert!(help().contains("/compactar") && help().contains("/deshacer"));
    }

    #[test]
    fn suggests_while_typing_the_name() {
        let names = |s: &str| suggestions(s).iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names("/co"), ["compactar", "coste"]);
        assert_eq!(names("/ca"), ["cancelar"]);
        assert_eq!(names("/re"), ["historial", "reintentar", "deshacer", "renombrar"]);
        assert!(names("/modelo ").is_empty());
        assert!(names("hola").is_empty());
        assert_eq!(names("/").len(), COMMANDS.len());
    }
}
