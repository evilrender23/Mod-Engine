//! `modengine store`: the asset store (OpenGameArt.org) from the command line, and what the
//! editor's coding agent runs to find, look at and import free assets. The store itself
//! (parsing, the polite client, cache, shared rate limit, zips, CREDITS.md) is the
//! `modengine-store` crate, the same code as the editor's Tienda; the import is
//! `modengine::import`, the same as `modengine import`.
//!
//! Licences: CC0 and licences that only ask for credit are imported (the credit goes to
//! CREDITS.md); share-alike / GPL only, or licences that can't be read, need
//! `--acepto-licencia` (exit code 4 without it).

use crate::import_cli::{describe, import_flag, item_json};
use modengine::import::{self as imp, human_size, Item, Kind, Options, What};
use modengine_store::credits::{self as cr, Credit};
use modengine_store::net::{hash, Client};
use modengine_store::oga::{self, allowed_url, blocking_note, license_notes, slug_of, ArtKind, Asset, FileLink, License, Note, Query, Sort, BASE};
use modengine_store::zip::{self as zip, ZipItem};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage:
  modengine store search <consulta...> [--tipo 2d|texturas|sonido|musica|fuentes]
                         [--licencia cc0,cc-by,oga-by,cc-by-sa,gpl,libres,todas]
                         [--orden favoritos|recientes|relevancia] [--pagina N] [--json]
      busca en OpenGameArt.org (por defecto: arte 2D con licencias sin «compartir igual»:
      CC0, CC-BY y OGA-BY)
  modengine store show <slug|url> [--zip] [--json]
      autor, licencias (y lo que exigen), descripción, etiquetas y ficheros de un asset;
      --zip descarga los .zip y lista lo que contienen
  modengine store download <slug|url> [fichero] [--zip-entry RUTA...] [--out DIR] [--json]
      descarga un fichero a la caché (o a DIR) sin importarlo, para mirarlo antes
      (modengine inspect-image <ruta>); de un .zip extrae las entradas pedidas
  modengine store get <slug|url> <proyecto> [--file NOMBRE... | --all] [--zip-entry RUTA...]
                      [opciones de import] [--acepto-licencia] [--dry-run] [--json]
      descarga e importa en el proyecto (sprites/, tilesets/, sounds/, fonts/) y añade el
      crédito a CREDITS.md (autor, licencias, URL, ficheros, fecha). Opciones de import:
      --kind --name --frames N --grid WxH --origin O --fps F --tile WxH --offset X,Y --sep X,Y
      --split-rows --row-names a,b --crop X,Y,W,H --name-prefix P --overwrite (modengine import --help)
    --file NOMBRE      el fichero del asset (nombre o número de `store show`; repetible)
    --all              todos los ficheros importables (y todo lo importable de sus .zip)
    --zip-entry RUTA   entrada de un .zip (ruta, nombre o comodín: 'Slime/*.png'; repetible)
    --acepto-licencia  importar aunque la licencia sea «compartir igual» / GPL o no se pueda
                       leer (solo si el usuario lo ha aceptado)

  Las peticiones van de una en una, como mucho una cada 2 s (compartido con el editor), con
  caché de 24 h en ~/.cache/modengine/store. Solo se piden páginas /content/, búsquedas y
  ficheros de opengameart.org (lo que permite su robots.txt).
  Códigos de salida: 0 bien, 1 error, 2 argumentos incorrectos, 4 licencia no aceptada.";

type Res = (i32, String, String);

fn bad(e: impl std::fmt::Display) -> Res {
    (2, String::new(), format!("modengine store: {e}\n\n{USAGE}\n"))
}

fn fail(json: bool, e: impl std::fmt::Display, client: &Client) -> Res {
    if json {
        let out = json!({ "ok": false, "errors": [e.to_string()], "network_requests": client.requests });
        return (1, pretty(&out), String::new());
    }
    (1, String::new(), format!("error: {e}\n"))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}

// ------------------------------------------------------------------ argument values

pub fn parse_kind(v: &str) -> Option<ArtKind> {
    Some(match v.trim().to_lowercase().as_str() {
        "2d" | "arte" | "arte-2d" | "sprites" | "sprite" => ArtKind::Art2D,
        "texturas" | "textura" | "texture" | "textures" => ArtKind::Texture,
        "sonido" | "sonidos" | "sfx" | "efectos" => ArtKind::Sfx,
        "musica" | "música" | "music" => ArtKind::Music,
        "fuentes" | "fuente" | "font" | "fonts" => ArtKind::Font,
        _ => return None,
    })
}

fn kind_key(k: ArtKind) -> &'static str {
    match k {
        ArtKind::Art2D => "2d",
        ArtKind::Texture => "texturas",
        ArtKind::Sfx => "sonido",
        ArtKind::Music => "musica",
        ArtKind::Font => "fuentes",
    }
}

/// `cc0,cc-by,oga-by` → the licences to filter by (families expand to every version).
pub fn parse_licenses(v: &str) -> Result<BTreeSet<License>, String> {
    let mut out = BTreeSet::new();
    for t in v.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()) {
        let add: Vec<License> = match t.as_str() {
            "cc0" | "dominio-publico" | "dominio-público" => vec![License::Cc0],
            "cc-by" | "ccby" => vec![License::CcBy3, License::CcBy4],
            "oga-by" => vec![License::OgaBy3, License::OgaBy4],
            "cc-by-sa" | "ccbysa" => vec![License::CcBySa3, License::CcBySa4],
            "gpl" => vec![License::Gpl2, License::Gpl3],
            "libres" | "sin-sa" | "por-defecto" => License::default_filter().into_iter().collect(),
            "todas" | "all" => License::FILTERS.to_vec(),
            k => {
                let k = if k.ends_with(".0") { k.to_string() } else { format!("{k}.0") };
                vec![License::from_key(&k).ok_or_else(|| format!("--licencia: '{t}' no es una licencia de OpenGameArt (cc0, cc-by, cc-by-4.0, oga-by, cc-by-sa, gpl, libres, todas)"))?]
            }
        };
        out.extend(add);
    }
    if out.is_empty() {
        return Err("--licencia: di al menos una (cc0, cc-by, oga-by...)".into());
    }
    Ok(out)
}

pub fn parse_sort(v: &str) -> Option<Sort> {
    Some(match v.trim().to_lowercase().as_str() {
        "favoritos" | "favorites" | "count" | "populares" => Sort::Favorites,
        "recientes" | "nuevos" | "newest" | "created" => Sort::Newest,
        "relevancia" | "relevance" | "score" => Sort::Relevance,
        _ => return None,
    })
}

fn sort_key(s: Sort) -> &'static str {
    match s {
        Sort::Favorites => "favoritos",
        Sort::Newest => "recientes",
        Sort::Relevance => "relevancia",
    }
}

/// `slime`, `/content/slime`, `https://opengameart.org/content/slime#x` → `slime`. A URL the
/// store may not ask for is refused (exit 1), anything else that is not a slug is bad input.
pub fn asset_slug(s: &str) -> Result<String, Res> {
    let s = s.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        let path_ok = s.strip_prefix(BASE).is_some_and(|p| p.starts_with("/content/"));
        if !allowed_url(s) || !path_ok {
            return Err((1, String::new(), format!("error: dirección no permitida para la tienda: {s} (solo páginas {BASE}/content/<asset>)\n")));
        }
    }
    slug_of(s).ok_or_else(|| bad(format!("'{s}' no es un asset de OpenGameArt (usa el slug de `store search` o la URL {BASE}/content/<asset>)")))
}

/// `*` and `?` wildcards, case-insensitive.
pub fn glob(pat: &str, s: &str) -> bool {
    fn m(p: &[char], s: &[char]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some(('*', rest)) => (0..=s.len()).any(|i| m(rest, &s[i..])),
            Some((c, rest)) => s.split_first().is_some_and(|(d, srest)| (*c == '?' || c == d) && m(rest, srest)),
        }
    }
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let s: Vec<char> = s.to_lowercase().chars().collect();
    m(&p, &s)
}

// ------------------------------------------------------------------ licences

fn license_json(l: &License) -> Value {
    json!({ "name": l.name(), "url": l.url(), "requires_credit": l.requires_credit(), "share_alike": l.share_alike(), "gpl": l.is_gpl(), "recognized": !matches!(l, License::Other(_)) })
}

fn note_key(n: Note) -> &'static str {
    match n {
        Note::Info => "info",
        Note::Warn => "warn",
        Note::Block => "block",
    }
}

fn note_mark(n: Note) -> &'static str {
    match n {
        Note::Info => "·",
        Note::Warn => "!",
        Note::Block => "✖",
    }
}

/// Why importing needs `--acepto-licencia` (`None`: it doesn't). A choice that includes a
/// licence without share-alike (CC0 or CC-BY…) is free to take that one.
pub fn license_gate(ls: &[License]) -> Option<String> {
    if let Some(b) = blocking_note(ls) {
        return Some(b);
    }
    if ls.iter().all(|l| l.share_alike() || matches!(l, License::Other(_))) {
        let notes: Vec<String> = license_notes(ls).into_iter().filter(|(n, _)| *n == Note::Warn).map(|(_, t)| t).collect();
        return Some(format!("Licencia «compartir igual» ({}): {}", names(ls), notes.join(" ")));
    }
    None
}

fn names(ls: &[License]) -> String {
    if ls.is_empty() {
        "desconocida".into()
    } else {
        ls.iter().map(License::name).collect::<Vec<_>>().join(" / ")
    }
}

// ------------------------------------------------------------------ files

fn file_kind(name: &str) -> Option<Kind> {
    Kind::of(Path::new(name))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Sprite => "sprite",
        Kind::Sound => "sound",
        Kind::Font => "font",
    }
}

fn file_json(f: &FileLink) -> Value {
    let kind = if f.is_zip() { Some("zip") } else { file_kind(&f.name).map(kind_name) };
    json!({ "name": f.name, "url": f.url, "size": f.size, "mime": f.mime, "kind": kind, "importable": file_kind(&f.name).is_some() })
}

fn entry_json(it: &ZipItem) -> Value {
    json!({ "path": it.name, "size": it.size, "kind": file_kind(it.file_name()).map(kind_name), "importable": it.importable() })
}

fn size_text(n: Option<u64>) -> String {
    n.map(human_size).unwrap_or_else(|| "¿tamaño?".into())
}

/// The file of `a` called `want` (name, case-insensitive name, URL file name or number).
fn find_file<'a>(a: &'a Asset, want: &str) -> Option<&'a FileLink> {
    let w = want.trim();
    if let Ok(n) = w.parse::<usize>() {
        if n >= 1 && n <= a.files.len() && !a.files.iter().any(|f| f.name == w) {
            return a.files.get(n - 1);
        }
    }
    let url_name = |f: &FileLink| modengine::import::percent_decode(f.url.rsplit('/').next().unwrap_or(""));
    a.files.iter().find(|f| f.name == w).or_else(|| a.files.iter().find(|f| f.name.eq_ignore_ascii_case(w) || url_name(f).eq_ignore_ascii_case(w)))
}

fn files_list(a: &Asset) -> String {
    let mut s = String::new();
    for (i, f) in a.files.iter().enumerate() {
        let what = if f.is_zip() {
            "zip".to_string()
        } else {
            match file_kind(&f.name) {
                Some(k) => format!("importable ({})", k.label()),
                None => format!("no importable (.{})", f.ext()),
            }
        };
        s += &format!("  {}. {}  {}  {what}\n", i + 1, f.name, size_text(f.size));
    }
    s
}

/// Which files of `a` to fetch: `--file`s, `--all`, or the only one.
fn choose_files<'a>(a: &'a Asset, want: &[String], all: bool) -> Result<Vec<&'a FileLink>, String> {
    if a.files.is_empty() {
        return Err(format!("«{}» no tiene ficheros para descargar en su página", a.title));
    }
    if !want.is_empty() {
        let mut out: Vec<&FileLink> = vec![];
        for w in want {
            match find_file(a, w) {
                Some(f) if !out.contains(&f) => out.push(f),
                Some(_) => {}
                None => return Err(format!("«{}» no tiene el fichero '{w}'. Ficheros:\n{}", a.title, files_list(a))),
            }
        }
        return Ok(out);
    }
    if all {
        let v: Vec<&FileLink> = a.files.iter().filter(|f| f.is_zip() || file_kind(&f.name).is_some()).collect();
        if v.is_empty() {
            return Err(format!("«{}» no tiene ficheros importables (PNG, WAV, OGG, TTF, OTF o .zip). Ficheros:\n{}", a.title, files_list(a)));
        }
        return Ok(v);
    }
    if a.files.len() == 1 {
        return Ok(vec![&a.files[0]]);
    }
    Err(format!("«{}» tiene {} ficheros: elige con --file NOMBRE (o --all):\n{}", a.title, a.files.len(), files_list(a)))
}

/// Entries of a zip to use: the ones matching `pats`, all importable ones (`all`), or the
/// only importable one.
fn choose_entries<'a>(zip_name: &str, items: &'a [ZipItem], pats: &[String], all: bool) -> Result<Vec<&'a ZipItem>, String> {
    let importable: Vec<&ZipItem> = items.iter().filter(|i| i.importable()).collect();
    let listing = |v: &[&ZipItem]| v.iter().map(|i| format!("  {}  {}\n", i.name, human_size(i.size))).collect::<String>();
    if !pats.is_empty() {
        let v: Vec<&ZipItem> = items.iter().filter(|i| pats.iter().any(|p| i.name == *p || glob(p, &i.name) || glob(p, i.file_name()))).collect();
        if v.is_empty() {
            return Err(format!("ninguna entrada de {zip_name} coincide con --zip-entry {}. Contiene:\n{}", pats.join(" "), listing(&items.iter().collect::<Vec<_>>())));
        }
        return Ok(v);
    }
    match importable.len() {
        0 => Err(format!("{zip_name} no contiene nada importable (PNG, WAV, OGG, TTF, OTF)")),
        1 => Ok(importable),
        _ if all => Ok(importable),
        n => Err(format!("{zip_name} contiene {n} ficheros importables: elige con --zip-entry RUTA (vale un comodín, 'carpeta/*.png') o usa --all:\n{}", listing(&importable))),
    }
}

/// Where the entries of a downloaded zip are extracted (in the cache, like the editor).
fn extract_dir(client: &Client, url: &str) -> PathBuf {
    match &client.cache {
        Some(c) => c.dir.join("extract").join(hash(url)),
        None => std::env::temp_dir().join("modengine-store").join(hash(url)),
    }
}

fn read_zip(path: &Path) -> Result<(Vec<u8>, Vec<ZipItem>), String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let items = zip::list(&b).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((b, items))
}

// ------------------------------------------------------------------ search

fn run_search(args: &[String], client: &mut Client) -> Res {
    let mut q = Query::default();
    let (mut json_out, mut words) = (false, vec![]);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut value = || {
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{a} necesita un valor"))
        };
        match a {
            "--tipo" | "--kind" | "--type" => match value().map(|v| (parse_kind(&v), v)) {
                Ok((Some(k), _)) => q.kind = k,
                Ok((None, v)) => return bad(format!("--tipo: '{v}' (2d, texturas, sonido, musica, fuentes)")),
                Err(e) => return bad(e),
            },
            "--licencia" | "--licencias" | "--license" => match value().and_then(|v| parse_licenses(&v)) {
                Ok(l) => q.licenses = l,
                Err(e) => return bad(e),
            },
            "--orden" | "--sort" => match value().map(|v| (parse_sort(&v), v)) {
                Ok((Some(s), _)) => q.sort = s,
                Ok((None, v)) => return bad(format!("--orden: '{v}' (favoritos, recientes, relevancia)")),
                Err(e) => return bad(e),
            },
            "--pagina" | "--página" | "--page" => match value().map(|v| (v.parse::<u32>().ok().filter(|n| *n >= 1), v)) {
                Ok((Some(n), _)) => q.page = n - 1,
                Ok((None, v)) => return bad(format!("--pagina: '{v}' (1, 2, 3...)")),
                Err(e) => return bad(e),
            },
            "--json" => json_out = true,
            f if f.starts_with("--") => return bad(format!("opción desconocida {f}")),
            w => words.push(w.to_string()),
        }
        i += 1;
    }
    q.keys = words.join(" ");
    let page = match client.search(&q) {
        Ok(p) => p,
        Err(e) => return fail(json_out, e, client),
    };
    let lic_names: Vec<String> = q.licenses.iter().map(License::name).collect();
    // licences are on the asset pages: only the ones already in the cache (no requests)
    let known: Vec<Option<Vec<License>>> = page.hits.iter().map(|h| client.cached_detail(&h.slug).map(|a| a.licenses)).collect();
    if json_out {
        let results: Vec<Value> = page
            .hits
            .iter()
            .zip(&known)
            .map(|(h, k)| json!({ "slug": h.slug, "title": h.title, "url": oga::content_url(&h.slug), "thumbnail": h.thumb, "licenses": k.as_ref().map(|ls| ls.iter().map(License::name).collect::<Vec<_>>()) }))
            .collect();
        let out = json!({
            "ok": true,
            "query": { "keys": q.keys, "tipo": kind_key(q.kind), "licencias": lic_names, "orden": sort_key(q.sort), "pagina": q.page + 1 },
            "page": page.page + 1,
            "last_page": page.last_page + 1,
            "results": results,
            "note": "Las licencias de cada asset están en su página: `modengine store show <slug>`. El filtro de licencias de la búsqueda ya excluye las demás.",
            "network_requests": client.requests,
        });
        return (0, pretty(&out), String::new());
    }
    let mut out = format!(
        "OpenGameArt: «{}» · {} · licencias {} · {} · página {} de {} ({} resultados)\n",
        q.keys,
        q.kind.label(),
        lic_names.join(", "),
        q.sort.label().to_lowercase(),
        page.page + 1,
        page.last_page + 1,
        page.hits.len()
    );
    if page.hits.is_empty() {
        out += "  (nada: prueba otras palabras, en inglés, u otro --tipo / --licencia)\n";
    }
    for (n, (h, k)) in page.hits.iter().zip(&known).enumerate() {
        let lic = k.as_ref().map(|ls| format!("  [{}]", names(ls))).unwrap_or_default();
        out += &format!("{:>3}. {} — {}{lic}\n", n + 1, h.slug, h.title);
    }
    out += "Detalles (licencia, autor, ficheros): modengine store show <slug>";
    if page.page < page.last_page {
        out += &format!(" · más resultados: --pagina {}", page.page + 2);
    }
    out += "\n";
    (0, out, String::new())
}

// ------------------------------------------------------------------ show

fn run_show(args: &[String], client: &mut Client) -> Res {
    let json_out = args.iter().any(|a| a == "--json");
    let with_zip = args.iter().any(|a| a == "--zip");
    if let Some(f) = args.iter().find(|a| a.starts_with("--") && *a != "--json" && *a != "--zip") {
        return bad(format!("opción desconocida {f}"));
    }
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let [arg] = pos[..] else { return bad("store show: di un asset (slug o URL)") };
    let slug = match asset_slug(arg) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let a = match client.detail(&slug) {
        Ok(a) => a,
        Err(e) => return fail(json_out, e, client),
    };
    // zip contents, on request (downloads the zips)
    let mut zips: BTreeMap<String, Result<Vec<ZipItem>, String>> = BTreeMap::new();
    if with_zip {
        for f in a.files.iter().filter(|f| f.is_zip()) {
            let r = client.download(f).and_then(|p| read_zip(&p).map(|(_, items)| items));
            zips.insert(f.url.clone(), r);
        }
    }
    let notes = license_notes(&a.licenses);
    let gate = license_gate(&a.licenses);
    let suggest = suggest_get(&a);
    if json_out {
        let files: Vec<Value> = a
            .files
            .iter()
            .map(|f| {
                let mut v = file_json(f);
                match zips.get(&f.url) {
                    Some(Ok(items)) => v["entries"] = json!(items.iter().map(entry_json).collect::<Vec<_>>()),
                    Some(Err(e)) => v["entries_error"] = json!(e),
                    None => {}
                }
                v
            })
            .collect();
        let out = json!({
            "ok": true,
            "slug": a.slug,
            "title": a.title,
            "url": a.url(),
            "author": a.author,
            "author_url": a.author_url,
            "collaborators": a.collaborators,
            "date": a.date,
            "art_types": a.art_types,
            "tags": a.tags,
            "licenses": a.licenses.iter().map(license_json).collect::<Vec<_>>(),
            "license_notes": notes.iter().map(|(n, t)| json!({ "level": note_key(*n), "text": t })).collect::<Vec<_>>(),
            "needs_license_acceptance": gate.is_some(),
            "license_acceptance_reason": gate,
            "description": a.description,
            "notice": a.notice,
            "previews": a.previews.iter().map(|p| json!({ "image": (!p.image.is_empty()).then_some(&p.image), "full": p.full })).collect::<Vec<_>>(),
            "files": files,
            "get": suggest,
            "network_requests": client.requests,
        });
        return (0, pretty(&out), String::new());
    }
    let mut out = format!("{} — {}\n{}\n", a.title, if a.author.is_empty() { "autor desconocido" } else { &a.author }, a.url());
    if let Some(u) = &a.author_url {
        out += &format!("Autor: {} ({u})\n", a.author);
    }
    if !a.collaborators.is_empty() {
        out += &format!("Colaboradores: {}\n", a.collaborators.join(", "));
    }
    if !a.date.is_empty() {
        out += &format!("Fecha: {}\n", a.date);
    }
    if !a.art_types.is_empty() || !a.tags.is_empty() {
        out += &format!("Tipo: {} · etiquetas: {}\n", a.art_types.join(", "), a.tags.join(", "));
    }
    out += &format!("Licencia: {}\n", names(&a.licenses));
    for (n, t) in &notes {
        out += &format!("  {} {t}\n", note_mark(*n));
    }
    if gate.is_some() {
        out += "  ✖ `store get` necesita --acepto-licencia (solo si el usuario acepta esas condiciones)\n";
    }
    if !a.notice.trim().is_empty() {
        out += &format!("Aviso del autor: {}\n", a.notice.trim().replace('\n', " "));
    }
    if !a.description.trim().is_empty() {
        let d: String = a.description.chars().take(1200).collect();
        let more = if a.description.chars().count() > 1200 { "…" } else { "" };
        out += &format!("Descripción:\n{}{more}\n", d.lines().map(|l| format!("  {l}\n")).collect::<String>().trim_end());
    }
    let previews: Vec<&str> = a.previews.iter().filter_map(|p| p.full.as_deref().or((!p.image.is_empty()).then_some(p.image.as_str()))).collect();
    if !previews.is_empty() {
        out += &format!("Vistas previas: {}\n", previews.join("  "));
    }
    out += "Ficheros:\n";
    out += &files_list(&a);
    for f in a.files.iter().filter(|f| f.is_zip()) {
        match zips.get(&f.url) {
            Some(Ok(items)) => {
                out += &format!("  {} contiene:\n", f.name);
                for it in items {
                    let what = match file_kind(it.file_name()) {
                        Some(k) => format!("importable ({})", k.label()),
                        None => "no importable".into(),
                    };
                    out += &format!("     {}  {}  {what}\n", it.name, human_size(it.size));
                }
            }
            Some(Err(e)) => out += &format!("  {}: {e}\n", f.name),
            None => {}
        }
    }
    if !with_zip && a.files.iter().any(FileLink::is_zip) {
        out += "  (--zip descarga los .zip y lista lo que contienen)\n";
    }
    out += &format!("Para importarlo: {suggest}\n");
    (0, out, String::new())
}

fn quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
        s.to_string()
    } else {
        format!("\"{s}\"")
    }
}

fn suggest_get(a: &Asset) -> String {
    let mut s = format!("modengine store get {} <proyecto>", a.slug);
    if a.files.len() > 1 {
        match a.files.iter().find(|f| f.is_zip() || file_kind(&f.name).is_some()) {
            Some(f) => s += &format!(" --file {}", quote(&f.name)),
            None => s += " --file <fichero>",
        }
    }
    if a.files.iter().any(FileLink::is_zip) {
        s += " [--zip-entry <ruta>]";
    }
    if license_gate(&a.licenses).is_some() {
        s += " --acepto-licencia";
    }
    s + " [--frames N | --grid WxH] [--origin ...] --json"
}

// ------------------------------------------------------------------ download

fn run_download(args: &[String], client: &mut Client) -> Res {
    let (mut json_out, mut out_dir, mut entries, mut pos) = (false, None, vec![], vec![]);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--json" => json_out = true,
            "--out" | "--zip-entry" | "--file" => {
                i += 1;
                let Some(v) = args.get(i).cloned() else { return bad(format!("{a} necesita un valor")) };
                match a {
                    "--out" => out_dir = Some(PathBuf::from(v)),
                    "--zip-entry" => entries.push(v),
                    _ => pos.push(v),
                }
            }
            f if f.starts_with("--") => return bad(format!("opción desconocida {f}")),
            p => pos.push(p.to_string()),
        }
        i += 1;
    }
    if pos.is_empty() || pos.len() > 2 {
        return bad("store download: di el asset y, si tiene varios, el fichero");
    }
    let slug = match asset_slug(&pos[0]) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let a = match client.detail(&slug) {
        Ok(a) => a,
        Err(e) => return fail(json_out, e, client),
    };
    let f = match choose_files(&a, &pos[1..], false) {
        Ok(v) => v[0].clone(),
        Err(e) => return (2, String::new(), format!("modengine store: {e}")),
    };
    let path = match client.download(&f) {
        Ok(p) => p,
        Err(e) => return fail(json_out, e, client),
    };
    let mut listed: Vec<ZipItem> = vec![];
    let mut extracted: Vec<PathBuf> = vec![];
    let mut result_path = path.clone();
    if f.is_zip() {
        let (bytes, items) = match read_zip(&path) {
            Ok(x) => x,
            Err(e) => return fail(json_out, e, client),
        };
        if !entries.is_empty() {
            let chosen = match choose_entries(&f.name, &items, &entries, false) {
                Ok(v) => v,
                Err(e) => return (2, String::new(), format!("modengine store: {e}\n")),
            };
            let dir = out_dir.clone().unwrap_or_else(|| extract_dir(client, &f.url));
            match zip::extract_to(&bytes, &chosen, &dir) {
                Ok(p) => extracted = p,
                Err(e) => return fail(json_out, e, client),
            }
        }
        listed = items;
    } else if let Some(dir) = &out_dir {
        let dst = dir.join(modengine_store::net::safe_file_name(&f.name));
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::copy(&path, &dst)) {
            return fail(json_out, format!("{}: {e}", dst.display()), client);
        }
        result_path = dst;
    }
    if json_out {
        let out = json!({
            "ok": true,
            "asset": { "slug": a.slug, "title": a.title, "licenses": a.licenses.iter().map(License::name).collect::<Vec<_>>() },
            "file": file_json(&f),
            "path": result_path.display().to_string(),
            "entries": listed.iter().map(entry_json).collect::<Vec<_>>(),
            "extracted": extracted.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "network_requests": client.requests,
        });
        return (0, pretty(&out), String::new());
    }
    let mut out = format!("Descargado {} ({}) de «{}»:\n  {}\n", f.name, size_text(std::fs::metadata(&path).ok().map(|m| m.len())), a.title, result_path.display());
    if !listed.is_empty() && extracted.is_empty() {
        out += "Contiene (extrae con --zip-entry RUTA):\n";
        for it in &listed {
            out += &format!("  {}  {}{}\n", it.name, human_size(it.size), if it.importable() { "" } else { "  (no importable)" });
        }
    }
    if !extracted.is_empty() {
        out += "Extraído:\n";
        for p in &extracted {
            out += &format!("  {}\n", p.display());
        }
    }
    out += "Míralo antes de importarlo: modengine inspect-image <ruta> (y ábrelo para verlo).\n";
    (0, out, String::new())
}

// ------------------------------------------------------------------ get

struct GetArgs {
    slug: String,
    project: PathBuf,
    files: Vec<String>,
    all: bool,
    entries: Vec<String>,
    accept: bool,
    dry_run: bool,
    json: bool,
    opts: Options,
}

fn parse_get(args: &[String]) -> Result<GetArgs, Res> {
    let mut g = GetArgs { slug: String::new(), project: PathBuf::new(), files: vec![], all: false, entries: vec![], accept: false, dry_run: false, json: false, opts: Options::default() };
    let mut pos = vec![];
    let mut i = 0;
    while i < args.len() {
        match import_flag(args, &mut i, &mut g.opts) {
            Ok(true) => {
                i += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => return Err(bad(e)),
        }
        let a = args[i].as_str();
        match a {
            "--file" | "--zip-entry" => {
                i += 1;
                let Some(v) = args.get(i).cloned() else { return Err(bad(format!("{a} necesita un valor"))) };
                if a == "--file" { g.files.push(v) } else { g.entries.push(v) }
            }
            "--all" | "--todo" | "--todos" => g.all = true,
            "--acepto-licencia" | "--accept-license" => g.accept = true,
            "--dry-run" | "-n" => g.dry_run = true,
            "--json" => g.json = true,
            "--credit" => return Err(bad("--credit no hace falta: el crédito de la Tienda se escribe solo en CREDITS.md")),
            f if f.starts_with("--") => return Err(bad(format!("opción desconocida {f}"))),
            p => pos.push(p.to_string()),
        }
        i += 1;
    }
    if g.opts.tile.is_some() && g.opts.kind.is_none() {
        g.opts.kind = Some(What::Tileset);
    }
    if pos.len() != 2 {
        return Err(bad("store get: di el asset (slug o URL) y la carpeta del proyecto"));
    }
    if g.all && !g.files.is_empty() {
        return Err(bad("--all y --file no van juntos"));
    }
    g.slug = asset_slug(&pos[0])?;
    g.project = PathBuf::from(&pos[1]);
    Ok(g)
}

fn run_get(args: &[String], client: &mut Client) -> Res {
    let g = match parse_get(args) {
        Ok(g) => g,
        Err(r) => return r,
    };
    // before any request: a wrong folder costs nothing
    if !g.project.join("game.toml").is_file() {
        return fail(g.json, format!("{} no es un proyecto de Mod Engine (falta game.toml)", g.project.display()), client);
    }
    let a = match client.detail(&g.slug) {
        Ok(a) => a,
        Err(e) => return fail(g.json, e, client),
    };
    let notes = license_notes(&a.licenses);
    let gate = license_gate(&a.licenses);
    if let (Some(why), false) = (&gate, g.accept) {
        let msg = format!(
            "«{}» no se importa sin aceptar su licencia ({}).\n{why}\nSi el usuario acepta esas condiciones, repite con --acepto-licencia. Licencia completa: {}\n",
            a.title,
            names(&a.licenses),
            a.url()
        );
        if g.json {
            let out = json!({ "ok": false, "license_not_accepted": true, "asset": asset_json(&a), "reason": why, "license_notes": notes.iter().map(|(n, t)| json!({ "level": note_key(*n), "text": t })).collect::<Vec<_>>(), "network_requests": client.requests });
            return (4, pretty(&out), String::new());
        }
        return (4, String::new(), msg);
    }
    let chosen = match choose_files(&a, &g.files, g.all) {
        Ok(v) => v,
        Err(e) => return (2, String::new(), format!("modengine store: {e}")),
    };
    if !g.entries.is_empty() && !chosen.iter().any(|f| f.is_zip()) {
        return bad("--zip-entry: los ficheros elegidos no son .zip");
    }
    // direct files must be something the engine loads (with --all they were filtered)
    if let Some(f) = chosen.iter().find(|f| !f.is_zip() && file_kind(&f.name).is_none()) {
        return fail(g.json, format!("{}: formato no soportado (.{}): el motor carga PNG, WAV, OGG, TTF y OTF", f.name, f.ext()), client);
    }
    // download, unzip: (file on disk, label for CREDITS.md)
    let mut inputs: Vec<(PathBuf, String)> = vec![];
    let mut downloads: Vec<Value> = vec![];
    let mut license_files: Vec<PathBuf> = vec![];
    for f in &chosen {
        let path = match client.download(f) {
            Ok(p) => p,
            Err(e) => return fail(g.json, format!("{}: {e}", f.name), client),
        };
        downloads.push(json!({ "name": f.name, "url": f.url, "path": path.display().to_string(), "size": std::fs::metadata(&path).map(|m| m.len()).ok() }));
        if !f.is_zip() {
            inputs.push((path, f.name.clone()));
            continue;
        }
        let (bytes, items) = match read_zip(&path) {
            Ok(x) => x,
            Err(e) => return fail(g.json, e, client),
        };
        let picked = match choose_entries(&f.name, &items, &g.entries, g.all) {
            Ok(v) => v,
            Err(e) => return (2, String::new(), format!("modengine store: {e}\n")),
        };
        // the pack's own licence / readme files go next to the art, for the agent to read
        let extra: Vec<&ZipItem> = items.iter().filter(|i| imp::is_license_file(i.file_name())).collect();
        let dir = extract_dir(client, &f.url);
        let all: Vec<&ZipItem> = picked.iter().copied().chain(extra.iter().copied().filter(|e| !picked.contains(e))).collect();
        let paths = match zip::extract_to(&bytes, &all, &dir) {
            Ok(p) => p,
            Err(e) => return fail(g.json, e, client),
        };
        for (it, p) in all.iter().zip(paths) {
            if picked.contains(it) {
                if it.importable() {
                    inputs.push((p, format!("{} → {}", f.name, it.name)));
                }
            } else {
                license_files.push(p);
            }
        }
    }
    if inputs.is_empty() {
        return fail(g.json, "no hay nada importable entre lo elegido (PNG, WAV, OGG, TTF u OTF)", client);
    }
    let srcs: Vec<PathBuf> = inputs.iter().map(|(p, _)| p.clone()).collect();
    let plan = imp::plan(&g.project, &srcs, &g.opts);
    let label_of = |src: &Path| inputs.iter().find(|(p, _)| p == src).map(|(_, l)| l.clone()).unwrap_or_default();
    let report = if g.dry_run || !plan.errors.is_empty() { imp::Report { errors: plan.errors.clone(), ..Default::default() } } else { imp::execute(&plan, None) };
    // CREDITS.md: one section per asset, a line per file with what it came from
    let mut credited = None;
    let mut errors = report.errors.clone();
    if !g.dry_run {
        let mut by_label: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (it, files, _) in &report.done {
            by_label.entry(label_of(&it.src)).or_default().extend(files.iter().cloned());
        }
        for (label, files) in by_label {
            match cr::write(&g.project, &Credit::from_asset(&a, &label), &files) {
                Ok(p) => credited = Some(p),
                Err(e) => errors.push(e),
            }
        }
    }
    let ok = errors.is_empty();
    let code = if ok { 0 } else { 1 };
    let list: Vec<(&Item, Vec<String>, bool)> = if g.dry_run { plan.items.iter().map(|it| (it, it.files(), it.replace)).collect() } else { report.done.iter().map(|(it, f, r)| (it, f.clone(), *r)).collect() };
    if g.json {
        let assets: Vec<Value> = list
            .iter()
            .map(|(it, files, r)| {
                let mut v = item_json(it, files, *r);
                v["from"] = json!(label_of(&it.src));
                v
            })
            .collect();
        let out = json!({
            "ok": ok,
            "dry_run": g.dry_run,
            "project": g.project.display().to_string(),
            "asset": asset_json(&a),
            "license_accepted": gate.is_some() && g.accept,
            "downloads": downloads,
            "assets": assets,
            "skipped": plan.skipped.iter().map(|(p, r)| json!({ "path": p.display().to_string(), "reason": r })).collect::<Vec<_>>(),
            "license_files": license_files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "credits": credited.as_ref().map(|_| cr::FILE),
            "license_notes": notes.iter().map(|(n, t)| json!({ "level": note_key(*n), "text": t })).collect::<Vec<_>>(),
            "errors": errors,
            "network_requests": client.requests,
        });
        return (code, pretty(&out), String::new());
    }
    let mut out = format!("De la Tienda: «{}» de {} ({}) {}\n", a.title, if a.author.is_empty() { "autor desconocido" } else { &a.author }, names(&a.licenses), a.url());
    if !list.is_empty() {
        out += &if g.dry_run { format!("Se importaría en {} (--dry-run: no se escribe nada):\n", g.project.display()) } else { format!("Importado en {}:\n", g.project.display()) };
        for (it, files, replaced) in &list {
            out += &format!("  {} {} {} ← {}  {}\n", if *replaced { "~" } else { "+" }, it.what.key(), it.name, label_of(&it.src), describe(it));
            out += &format!("      {}\n", files.join(", "));
            for w in &it.warnings {
                out += &format!("      aviso: {w}\n");
            }
            if !it.hint.is_empty() {
                out += &format!("      ¿una hoja de frames? {}\n", it.hint);
            }
        }
    }
    if credited.is_some() {
        let must = a.licenses.is_empty() || a.licenses.iter().all(License::requires_credit);
        out += &format!("{} actualizado{}\n", cr::FILE, if must { ": esta licencia exige mostrar el crédito en tu juego (pantalla de créditos, web o README)" } else { "" });
    }
    for (n, t) in notes.iter().filter(|(n, _)| *n != Note::Info) {
        out += &format!("  {} {t}\n", note_mark(*n));
    }
    if !license_files.is_empty() {
        out += "Ficheros de licencia / léeme del pack:\n";
        for p in &license_files {
            out += &format!("  {}\n", p.display());
        }
    }
    let err: String = errors.iter().map(|e| format!("error: {e}\n")).collect();
    (code, out, err)
}

fn asset_json(a: &Asset) -> Value {
    json!({ "slug": a.slug, "title": a.title, "author": a.author, "url": a.url(), "licenses": a.licenses.iter().map(License::name).collect::<Vec<_>>() })
}

// ------------------------------------------------------------------ entry points

/// Runs `modengine store <args>` with `client`; returns the exit code, stdout and stderr.
pub fn run(args: &[String], client: &mut Client) -> Res {
    let Some(sub) = args.first().map(String::as_str) else { return (0, USAGE.to_string() + "\n", String::new()) };
    let rest = &args[1..];
    if matches!(sub, "-h" | "--help" | "help") || rest.iter().any(|a| a == "-h" || a == "--help") {
        return (0, USAGE.to_string() + "\n", String::new());
    }
    match sub {
        "search" | "buscar" => run_search(rest, client),
        "show" | "ver" => run_show(rest, client),
        "get" | "importar" => run_get(rest, client),
        "download" | "descargar" => run_download(rest, client),
        s => bad(format!("subcomando desconocido '{s}' (search, show, download, get)")),
    }
}

/// `modengine store ...` from `main`.
pub fn main_cli(args: &[String]) -> i32 {
    let mut c = Client::real();
    let mut told = false;
    c.on_wait = Some(Box::new(move || {
        if !told {
            eprintln!("(esperando turno: la Tienda hace como mucho una petición cada 2 s a opengameart.org)");
            told = true;
        }
    }));
    let (code, out, err) = run(args, &mut c);
    print!("{out}");
    eprint!("{err}");
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use modengine_store::net::DiskCache;
    use modengine_store::testing::{fixture, make_zip, tmp, Mock};
    use std::sync::Arc;
    use std::time::Duration;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn project(dir: &Path) -> PathBuf {
        let p = dir.join("game");
        modengine::scaffold::create_with(&p, &modengine::scaffold::NewProject { template: "empty".into(), ..Default::default() }).unwrap();
        p
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = std::io::Cursor::new(vec![]);
        image::RgbaImage::from_fn(w, h, |x, _| if x % 16 > 2 && x % 16 < 13 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) }).write_to(&mut b, image::ImageFormat::Png).unwrap();
        b.into_inner()
    }

    /// The real pages of the fixtures with files the size the pages announce.
    fn store(dir: &Path) -> (Arc<Mock>, Client) {
        let mut m = Mock::default();
        for slug in ["animated-slime", "pixel-art-animated-slime", "tiny-creatures"] {
            m.pages.insert(oga::content_url(slug), Ok(fixture(&format!("{slug}.html"))));
        }
        m.pages.insert(Query { keys: "slime".into(), licenses: [License::Cc0].into_iter().collect(), ..Default::default() }.url(), Ok(fixture("search_slime_cc0.html")));
        // animated-slime: one 12041-byte PNG (a strip of 4 frames, padded to the size)
        let mut sheet = png(64, 16);
        sheet.resize(12041, 0);
        m.pages.insert("https://opengameart.org/sites/default/files/slime%20spritesheet%20calciumtrice_0.png".into(), Ok(sheet));
        // pixel-art-animated-slime: Slime.zip of 12061 bytes
        let mut z = make_zip(&[("Slime/slime_idle_strip4.png", &png(64, 16), true), ("Slime/slime_move.png", &png(32, 16), true), ("Slime/LICENSE.txt", b"CC0 by rvros", false)]);
        assert!(z.len() < 12061);
        // pad inside the zip comment is not possible with this writer: append and fix the size
        let pad = 12061 - z.len();
        let n = z.len();
        z[n - 2..].copy_from_slice(&(pad as u16).to_le_bytes());
        z.resize(12061, b' ');
        let zu = parse_zip_url(&fixture("pixel-art-animated-slime.html"));
        m.pages.insert(zu, Ok(z));
        let m = Arc::new(m);
        let c = Client::new(m.clone(), Some(DiskCache::new(dir.join("cache"), modengine_store::net::CACHE_TTL)), Duration::ZERO);
        (m, c)
    }

    fn parse_zip_url(html: &[u8]) -> String {
        oga::parse_detail(&String::from_utf8_lossy(html), "pixel-art-animated-slime").unwrap().files[0].url.clone()
    }

    #[test]
    fn argument_values() {
        assert_eq!(parse_kind("musica"), Some(ArtKind::Music));
        assert_eq!(parse_kind("2d"), Some(ArtKind::Art2D));
        assert_eq!(parse_kind("3d"), None);
        assert_eq!(parse_licenses("cc0").unwrap(), [License::Cc0].into_iter().collect());
        assert_eq!(parse_licenses("cc-by, oga-by").unwrap().len(), 4);
        assert_eq!(parse_licenses("cc-by-4.0").unwrap(), [License::CcBy4].into_iter().collect());
        assert_eq!(parse_licenses("cc-by-sa-3").unwrap(), [License::CcBySa3].into_iter().collect());
        assert_eq!(parse_licenses("libres").unwrap(), License::default_filter());
        assert!(parse_licenses("mit").is_err() && parse_licenses(",").is_err());
        assert_eq!(parse_sort("recientes"), Some(Sort::Newest));
        assert_eq!(asset_slug("https://opengameart.org/content/animated-slime#comments").unwrap(), "animated-slime");
        assert_eq!(asset_slug("animated-slime").unwrap(), "animated-slime");
        assert_eq!(asset_slug("https://opengameart.org/users/x").unwrap_err().0, 1, "not allowed: refused");
        assert_eq!(asset_slug("https://evil.example/content/x").unwrap_err().0, 1);
        assert_eq!(asset_slug("a/b c").unwrap_err().0, 2);
        assert!(glob("Slime/*.png", "slime/slime_idle.png") && glob("*idle*", "Slime/slime_idle.png") && !glob("*.wav", "a.png"));
        assert!(license_gate(&[License::Cc0]).is_none() && license_gate(&[License::CcBy3]).is_none());
        assert!(license_gate(&[License::CcBySa4]).unwrap().contains("compartir igual"));
        assert!(license_gate(&[License::Gpl3]).is_some() && license_gate(&[]).is_some() && license_gate(&[License::Other("x".into())]).is_some());
        assert!(license_gate(&[License::Cc0, License::CcBySa3]).is_none(), "the user can pick CC0");
        let mut c = Client::new(Arc::new(Mock::default()), None, Duration::ZERO);
        assert_eq!(run(&s(&["search", "--tipo", "3d", "x"]), &mut c).0, 2);
        assert_eq!(run(&s(&["search", "--pagina", "0", "x"]), &mut c).0, 2);
        assert_eq!(run(&s(&["show"]), &mut c).0, 2);
        assert_eq!(run(&s(&["get", "x"]), &mut c).0, 2);
        assert_eq!(run(&s(&["get", "x", "p", "--frames", "zz"]), &mut c).0, 2);
        assert_eq!(run(&s(&["get", "x", "p", "--all", "--file", "a"]), &mut c).0, 2);
        assert_eq!(run(&s(&["nope"]), &mut c).0, 2);
        assert_eq!(run(&s(&["--help"]), &mut c).0, 0);
        assert_eq!(c.requests, 0, "bad arguments never reach the network");
    }

    #[test]
    fn search_and_show_from_fixtures() {
        let dir = tmp("cli_show");
        let (m, mut c) = store(&dir);
        let (code, out, err) = run(&s(&["search", "slime", "--licencia", "cc0"]), &mut c);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("pixel-art-animated-slime — Pixel art animated Slime") && out.contains("página 1 de 4") && out.contains("--pagina 2"), "{out}");
        let (code, out, _) = run(&s(&["search", "slime", "--licencia", "cc0", "--json"]), &mut c);
        assert_eq!(code, 0);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["results"].as_array().unwrap().len(), 24);
        assert_eq!(v["query"]["licencias"], json!(["CC0"]));
        assert_eq!((v["page"].as_u64(), v["last_page"].as_u64()), (Some(1), Some(4)));
        assert!(v["results"][0]["thumbnail"].as_str().unwrap().starts_with("https://opengameart.org/sites/default/files/"));
        assert_eq!(v["network_requests"], 1, "the second search came from the cache (count is per client)");
        // show: human and json
        let (code, out, _) = run(&s(&["show", "https://opengameart.org/content/animated-slime"]), &mut c);
        assert_eq!(code, 0);
        assert!(out.starts_with("Animated Slime — Calciumtrice\n") && out.contains("Licencia: CC-BY 3.0") && out.contains("! Exige dar crédito"), "{out}");
        assert!(out.contains("1. slime spritesheet calciumtrice.png  11.8 KB  importable (sprite)"), "{out}");
        let (_, out, _) = run(&s(&["show", "animated-slime", "--json"]), &mut c);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["licenses"][0]["requires_credit"], true);
        assert_eq!(v["needs_license_acceptance"], false);
        assert_eq!(v["files"][0]["kind"], "sprite");
        assert_eq!(v["tags"], json!(["Pixel Art", "monster", "slime"]));
        // now the search knows its licence from the cached page
        let (_, out, _) = run(&s(&["search", "slime", "--licencia", "cc0", "--json"]), &mut c);
        assert!(out.contains("\"licenses\": null"));
        // --zip lists the zip contents
        let (code, out, _) = run(&s(&["show", "pixel-art-animated-slime", "--zip", "--json"]), &mut c);
        assert_eq!(code, 0, "{out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        let entries: Vec<&str> = v["files"][0]["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
        assert_eq!(entries, ["Slime/slime_idle_strip4.png", "Slime/slime_move.png", "Slime/LICENSE.txt"]);
        let hits = m.hits.lock().unwrap().len();
        assert_eq!(hits, 4, "search, two pages, one zip: {:?}", m.hits.lock().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_imports_with_credits_once() {
        let dir = tmp("cli_get");
        let p = project(&dir);
        let (_, mut c) = store(&dir);
        let ps = p.to_string_lossy().into_owned();
        let (code, out, err) = run(&s(&["get", "animated-slime", &ps, "--frames", "4", "--name", "slime", "--origin", "bottom-center", "--json"]), &mut c);
        assert_eq!(code, 0, "{out}{err}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["assets"][0]["name"], "slime");
        assert_eq!(v["assets"][0]["frames"], 4);
        assert_eq!(v["credits"], "CREDITS.md");
        let game = modengine::project::Project::load(&p).unwrap();
        let spr = game.sprites.iter().find(|s| s.name == "slime").unwrap();
        assert_eq!((spr.frames.len(), spr.origin), (4, (8.0, 16.0)));
        let md = std::fs::read_to_string(p.join("CREDITS.md")).unwrap();
        assert!(md.contains("## Animated Slime — Calciumtrice") && md.contains("[CC-BY 3.0](https://creativecommons.org/licenses/by/3.0/)") && md.contains("<!-- oga:animated-slime -->"), "{md}");
        assert!(md.contains("- Fichero: `sprites/slime.png` (de «slime spritesheet calciumtrice.png»"));
        assert!(std::fs::read_to_string(p.join("sprites/slime.toml")).unwrap().contains("# Origen: «Animated Slime»"));
        // again with --overwrite: still one section, no repeated lines
        let (code, out, err) = run(&s(&["get", "animated-slime", &ps, "--frames", "4", "--name", "slime", "--overwrite"]), &mut c);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("Importado en") && out.contains("exige mostrar el crédito"), "{out}");
        let md2 = std::fs::read_to_string(p.join("CREDITS.md")).unwrap();
        assert_eq!(md2.matches("<!-- oga:animated-slime -->").count(), 1);
        assert_eq!(md2.matches("`sprites/slime.png`").count(), 1);
        // dry run writes nothing
        let (code, out, _) = run(&s(&["get", "animated-slime", &ps, "--name", "otro", "--dry-run"]), &mut c);
        assert_eq!(code, 0);
        assert!(out.contains("--dry-run") && !p.join("sprites/otro.png").exists());
        // not a project: no request at all
        let before = c.requests;
        assert_eq!(run(&s(&["get", "animated-slime", dir.to_str().unwrap()]), &mut c).0, 1);
        assert_eq!(c.requests, before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_takes_zip_entries() {
        let dir = tmp("cli_zip");
        let p = project(&dir);
        let (_, mut c) = store(&dir);
        let ps = p.to_string_lossy().into_owned();
        // two importable entries: it asks which
        let (code, _, err) = run(&s(&["get", "pixel-art-animated-slime", &ps]), &mut c);
        assert_eq!(code, 2);
        assert!(err.contains("--zip-entry") && err.contains("Slime/slime_move.png"), "{err}");
        let (code, out, err) = run(&s(&["get", "pixel-art-animated-slime", &ps, "--zip-entry", "Slime/*idle*", "--json"]), &mut c);
        assert_eq!(code, 0, "{out}{err}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["assets"][0]["name"], "slime_idle");
        assert_eq!(v["assets"][0]["frames"], 4, "the _strip4 name sets the frames");
        assert_eq!(v["assets"][0]["from"], "Slime.zip → Slime/slime_idle_strip4.png");
        assert!(v["license_files"][0].as_str().unwrap().ends_with("LICENSE.txt"));
        let md = std::fs::read_to_string(p.join("CREDITS.md")).unwrap();
        assert!(md.contains("(de «Slime.zip → Slime/slime_idle_strip4.png»") && md.contains("[CC0]"), "{md}");
        // --all: everything importable in the zip, credited in the same section
        let (code, out, err) = run(&s(&["get", "pixel-art-animated-slime", &ps, "--all"]), &mut c);
        assert_eq!(code, 0, "{out}{err}");
        let md = std::fs::read_to_string(p.join("CREDITS.md")).unwrap();
        assert_eq!(md.matches("<!-- oga:pixel-art-animated-slime -->").count(), 1);
        assert!(md.contains("`sprites/slime_move.png`"));
        assert!(modengine::project::Project::load(&p).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn share_alike_needs_acceptance() {
        let dir = tmp("cli_sa");
        let p = project(&dir);
        let (m, c) = store(&dir);
        // the same page with a CC-BY-SA licence
        let html = String::from_utf8(fixture("animated-slime.html")).unwrap().replace("CC-BY 3.0", "CC-BY-SA 4.0");
        let mut m2 = Mock::default();
        for (k, v) in m.pages.iter() {
            m2.pages.insert(k.clone(), v.clone());
        }
        m2.pages.insert(oga::content_url("animated-slime"), Ok(html.into_bytes()));
        let mut c2 = Client::new(Arc::new(m2), Some(DiskCache::new(dir.join("cache2"), modengine_store::net::CACHE_TTL)), Duration::ZERO);
        let ps = p.to_string_lossy().into_owned();
        let (code, out, err) = run(&s(&["get", "animated-slime", &ps, "--frames", "4"]), &mut c2);
        assert_eq!(code, 4, "{out}{err}");
        assert!(err.contains("--acepto-licencia") && err.contains("misma licencia"), "{err}");
        assert!(!p.join("CREDITS.md").exists() && c2.requests == 1, "nothing downloaded");
        let (code, out, _) = run(&s(&["get", "animated-slime", &ps, "--json"]), &mut c2);
        assert_eq!(code, 4);
        assert!(out.contains("\"license_not_accepted\": true"));
        let (code, out, err) = run(&s(&["get", "animated-slime", &ps, "--frames", "4", "--acepto-licencia", "--json"]), &mut c2);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("\"license_accepted\": true"));
        assert!(std::fs::read_to_string(p.join("CREDITS.md")).unwrap().contains("CC-BY-SA 4.0"));
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_to_look_first() {
        let dir = tmp("cli_dl");
        let (_, mut c) = store(&dir);
        let (code, out, err) = run(&s(&["download", "animated-slime", "--json"]), &mut c);
        assert_eq!(code, 0, "{err}");
        let v: Value = serde_json::from_str(&out).unwrap();
        let path = PathBuf::from(v["path"].as_str().unwrap());
        assert!(path.starts_with(dir.join("cache")) && path.is_file());
        let out_dir = dir.join("mirar");
        let (code, out, err) = run(&s(&["download", "pixel-art-animated-slime", "Slime.zip", "--zip-entry", "*move*", "--out", out_dir.to_str().unwrap()]), &mut c);
        assert_eq!(code, 0, "{err}");
        assert!(out_dir.join("slime_move.png").is_file(), "{out}");
        assert_eq!(run(&s(&["download", "pixel-art-animated-slime", "nope.png"]), &mut c).0, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
