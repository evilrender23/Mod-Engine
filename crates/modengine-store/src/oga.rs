//! OpenGameArt.org for the asset store: search URLs, licences and a small HTML extractor for
//! the two kinds of page the store reads (`/art-search-advanced` results and `/content/<slug>`
//! asset pages). OGA is a Drupal 7 site without a public JSON API; the markup is regular
//! (`field field-name-…` blocks), so a careful hand-written extractor is enough and keeps the
//! editor free of HTML-parsing crates. Tests run on trimmed copies of real pages
//! (`tests/fixtures/oga/`).
//!
//! robots.txt allows `/content/` and `/art-search-advanced` (and the files under
//! `/sites/default/files/`) and disallows `/search/`: [`allowed_url`] is the gate every request
//! goes through.

use std::collections::BTreeSet;

pub const BASE: &str = "https://opengameart.org";

/// Every request the store may make. Anything else (other hosts, `/search/`, user pages…)
/// is refused before reaching the network.
pub fn allowed_url(url: &str) -> bool {
    let Some(path) = url.strip_prefix(BASE) else { return false };
    if path.contains("..") || path.contains(char::is_whitespace) {
        return false;
    }
    path.starts_with("/art-search-advanced?") || path.starts_with("/content/") || path.starts_with("/sites/default/files/")
}

// ------------------------------------------------------------------ licences

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum License {
    Cc0,
    CcBy3,
    CcBy4,
    OgaBy3,
    OgaBy4,
    CcBySa3,
    CcBySa4,
    Gpl2,
    Gpl3,
    Lgpl2,
    Lgpl3,
    /// Shown on the page but not recognised (older CC versions, custom text…).
    Other(String),
}

impl License {
    /// The licences the search form can filter by (taxonomy ids checked on the live form).
    pub const FILTERS: [License; 9] = [
        License::Cc0,
        License::CcBy3,
        License::CcBy4,
        License::OgaBy3,
        License::OgaBy4,
        License::CcBySa3,
        License::CcBySa4,
        License::Gpl2,
        License::Gpl3,
    ];

    /// Store default: no share-alike nor GPL obligations.
    pub fn default_filter() -> BTreeSet<License> {
        [License::Cc0, License::CcBy3, License::CcBy4, License::OgaBy3, License::OgaBy4].into_iter().collect()
    }

    /// `field_art_licenses_tid[]` value.
    pub fn tid(&self) -> Option<u32> {
        Some(match self {
            License::Cc0 => 4,
            License::CcBy3 => 2,
            License::CcBy4 => 17981,
            License::OgaBy3 => 10310,
            License::OgaBy4 => 31772,
            License::CcBySa3 => 3,
            License::CcBySa4 => 17982,
            License::Gpl2 => 5,
            License::Gpl3 => 6,
            _ => return None,
        })
    }

    /// The name OGA shows (`CC-BY 3.0`).
    pub fn name(&self) -> String {
        match self {
            License::Cc0 => "CC0".into(),
            License::CcBy3 => "CC-BY 3.0".into(),
            License::CcBy4 => "CC-BY 4.0".into(),
            License::OgaBy3 => "OGA-BY 3.0".into(),
            License::OgaBy4 => "OGA-BY 4.0".into(),
            License::CcBySa3 => "CC-BY-SA 3.0".into(),
            License::CcBySa4 => "CC-BY-SA 4.0".into(),
            License::Gpl2 => "GPL 2.0".into(),
            License::Gpl3 => "GPL 3.0".into(),
            License::Lgpl2 => "LGPL 2.1".into(),
            License::Lgpl3 => "LGPL 3.0".into(),
            License::Other(s) => s.clone(),
        }
    }

    /// Short key for store.toml (`cc-by-3.0`).
    pub fn key(&self) -> String {
        self.name().to_lowercase().replace(' ', "-")
    }

    pub fn from_key(k: &str) -> Option<License> {
        License::FILTERS.into_iter().find(|l| l.key() == k)
    }

    /// The licence text.
    pub fn url(&self) -> Option<&'static str> {
        Some(match self {
            License::Cc0 => "https://creativecommons.org/publicdomain/zero/1.0/",
            License::CcBy3 => "https://creativecommons.org/licenses/by/3.0/",
            License::CcBy4 => "https://creativecommons.org/licenses/by/4.0/",
            License::OgaBy3 => "https://static.opengameart.org/OGA-BY-3.0.txt",
            License::OgaBy4 => "https://static.opengameart.org/OGA-BY-4.0.txt",
            License::CcBySa3 => "https://creativecommons.org/licenses/by-sa/3.0/",
            License::CcBySa4 => "https://creativecommons.org/licenses/by-sa/4.0/",
            License::Gpl2 => "https://www.gnu.org/licenses/old-licenses/gpl-2.0.html",
            License::Gpl3 => "https://www.gnu.org/licenses/gpl-3.0.html",
            License::Lgpl2 => "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html",
            License::Lgpl3 => "https://www.gnu.org/licenses/lgpl-3.0.html",
            License::Other(_) => return None,
        })
    }

    /// From the name on the page: `CC-BY 3.0`, `CC-BY-SA 4.0`, `OGA-BY 3.0`, `GPL 2.0`…
    pub fn parse(name: &str) -> License {
        let n: String = name.trim().to_uppercase().chars().filter(|c| !c.is_whitespace()).collect();
        let n = n.replace('_', "-");
        match n.as_str() {
            "CC0" | "CC0-1.0" | "CC01.0" | "PUBLICDOMAIN" => License::Cc0,
            "CC-BY3.0" | "CC-BY-3.0" => License::CcBy3,
            "CC-BY4.0" | "CC-BY-4.0" => License::CcBy4,
            "OGA-BY3.0" | "OGA-BY-3.0" => License::OgaBy3,
            "OGA-BY4.0" | "OGA-BY-4.0" => License::OgaBy4,
            "CC-BY-SA3.0" | "CC-BY-SA-3.0" => License::CcBySa3,
            "CC-BY-SA4.0" | "CC-BY-SA-4.0" => License::CcBySa4,
            "GPL2.0" | "GPL-2.0" | "GPLV2" => License::Gpl2,
            "GPL3.0" | "GPL-3.0" | "GPLV3" => License::Gpl3,
            "LGPL2.1" | "LGPL-2.1" | "LGPL2.0" => License::Lgpl2,
            "LGPL3.0" | "LGPL-3.0" => License::Lgpl3,
            _ => License::Other(name.trim().to_string()),
        }
    }

    /// The author has to be credited.
    pub fn requires_credit(&self) -> bool {
        !matches!(self, License::Cc0)
    }

    /// Modified versions must keep the same licence (CC-BY-SA, GPL, LGPL).
    pub fn share_alike(&self) -> bool {
        matches!(self, License::CcBySa3 | License::CcBySa4 | License::Gpl2 | License::Gpl3 | License::Lgpl2 | License::Lgpl3)
    }

    pub fn is_gpl(&self) -> bool {
        matches!(self, License::Gpl2 | License::Gpl3 | License::Lgpl2 | License::Lgpl3)
    }

    /// Badge colour group: 0 free, 1 credit, 2 share-alike / copyleft, 3 unknown.
    pub fn level(&self) -> u8 {
        match self {
            License::Cc0 => 0,
            License::Other(_) => 3,
            l if l.share_alike() => 2,
            _ => 1,
        }
    }
}

/// How serious a licence note is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Info,
    Warn,
    /// Can't be imported without an explicit acknowledgement.
    Block,
}

/// What the licences of an asset ask from the user, in Spanish, most serious first.
pub fn license_notes(ls: &[License]) -> Vec<(Note, String)> {
    let mut out = vec![];
    if ls.is_empty() {
        out.push((Note::Block, "No se pudo leer la licencia de la página: revísala en el navegador antes de usar este asset.".into()));
        return out;
    }
    let unknown: Vec<String> = ls.iter().filter(|l| matches!(l, License::Other(_))).map(License::name).collect();
    if !unknown.is_empty() && unknown.len() == ls.len() {
        out.push((Note::Block, format!("Licencia no reconocida ({}): léela en la página original antes de usar este asset.", unknown.join(", "))));
    }
    if ls.len() > 1 {
        out.push((Note::Info, "Tiene varias licencias: puedes usarlo bajo la que prefieras (en CREDITS.md se anotan todas).".into()));
    }
    let any = |f: fn(&License) -> bool| ls.iter().any(f);
    let all = |f: fn(&License) -> bool| ls.iter().all(f);
    if all(|l| *l == License::Cc0) {
        out.push((Note::Info, "CC0 (dominio público): no exige crédito, aunque se agradece. Se anota igualmente en CREDITS.md.".into()));
        return out;
    }
    if any(|l| l.requires_credit() && !matches!(l, License::Other(_))) && !any(|l| *l == License::Cc0) {
        out.push((Note::Warn, "Exige dar crédito al autor: se añade a CREDITS.md y tienes que mostrarlo en tu juego (pantalla de créditos, web o README).".into()));
    }
    if all(License::share_alike) {
        if any(License::is_gpl) {
            out.push((Note::Warn, "GPL/LGPL: si distribuyes el asset (o una versión modificada) tiene que ir bajo la misma licencia, con su texto y acceso a los ficheros fuente. Puede no ser compatible con algunas tiendas.".into()));
        }
        if any(|l| matches!(l, License::CcBySa3 | License::CcBySa4)) {
            out.push((Note::Warn, "CC-BY-SA (compartir igual): si modificas el asset, tu versión debe publicarse con la misma licencia. El resto de tu juego no queda afectado.".into()));
        }
    }
    if any(|l| matches!(l, License::OgaBy3 | License::OgaBy4)) {
        out.push((Note::Info, "OGA-BY es como CC-BY pero permite DRM (p. ej. tiendas de consola): crédito obligatorio.".into()));
    }
    out
}

/// The note that stops an import until the user confirms, if any.
pub fn blocking_note(ls: &[License]) -> Option<String> {
    license_notes(ls).into_iter().find(|(n, _)| *n == Note::Block).map(|(_, s)| s)
}

// ------------------------------------------------------------------ search queries

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtKind {
    Art2D,
    Texture,
    Sfx,
    Music,
    /// No such art type on OGA: assets tagged `font`.
    Font,
}

impl ArtKind {
    pub const ALL: [ArtKind; 5] = [ArtKind::Art2D, ArtKind::Texture, ArtKind::Sfx, ArtKind::Music, ArtKind::Font];

    /// `field_art_type_tid[]` (3D art is 10 and never offered: the engine is 2D).
    pub fn tid(self) -> Option<u32> {
        match self {
            ArtKind::Art2D => Some(9),
            ArtKind::Texture => Some(14),
            ArtKind::Sfx => Some(13),
            ArtKind::Music => Some(12),
            ArtKind::Font => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ArtKind::Art2D => "Arte 2D",
            ArtKind::Texture => "Texturas",
            ArtKind::Sfx => "Efectos de sonido",
            ArtKind::Music => "Música",
            ArtKind::Font => "Fuentes",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ArtKind::Art2D => "2d",
            ArtKind::Texture => "texture",
            ArtKind::Sfx => "sfx",
            ArtKind::Music => "music",
            ArtKind::Font => "font",
        }
    }

    pub fn from_key(k: &str) -> Option<ArtKind> {
        ArtKind::ALL.into_iter().find(|a| a.key() == k)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Favorites,
    Newest,
    Relevance,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Favorites, Sort::Newest, Sort::Relevance];

    pub fn key(self) -> &'static str {
        match self {
            Sort::Favorites => "count",
            Sort::Newest => "created",
            Sort::Relevance => "score",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Favorites => "Más favoritos",
            Sort::Newest => "Más recientes",
            Sort::Relevance => "Relevancia",
        }
    }

    pub fn from_key(k: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.key() == k)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub keys: String,
    pub kind: ArtKind,
    pub licenses: BTreeSet<License>,
    pub sort: Sort,
    /// 0-based.
    pub page: u32,
}

impl Default for Query {
    fn default() -> Self {
        Query { keys: String::new(), kind: ArtKind::Art2D, licenses: License::default_filter(), sort: Sort::Favorites, page: 0 }
    }
}

impl Query {
    pub fn url(&self) -> String {
        let mut u = format!("{BASE}/art-search-advanced?keys={}", encode(self.keys.trim()));
        if let Some(t) = self.kind.tid() {
            u += &format!("&field_art_type_tid%5B%5D={t}");
        }
        if self.kind == ArtKind::Font {
            u += "&field_art_tags_tid=font";
        }
        for l in &self.licenses {
            if let Some(t) = l.tid() {
                u += &format!("&field_art_licenses_tid%5B%5D={t}");
            }
        }
        u += &format!("&sort_by={}&sort_order=DESC&page={}", self.sort.key(), self.page);
        u
    }
}

/// Query-string encoding (spaces as `+`).
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out += &format!("%{b:02X}"),
        }
    }
    out
}

pub fn content_url(slug: &str) -> String {
    format!("{BASE}/content/{slug}")
}

/// A slug as it appears in `/content/<slug>` links (no slashes, queries or fragments).
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() < 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.%".contains(&b)) && !s.contains("..")
}

/// `https://opengameart.org/content/x`, `/content/x#comments`, `x` → `x`.
pub fn slug_of(href: &str) -> Option<String> {
    let rest = href.strip_prefix(BASE).unwrap_or(href);
    let rest = rest.strip_prefix("/content/").or_else(|| (!rest.contains('/')).then_some(rest))?;
    let s = rest.split(['#', '?', '/']).next()?.to_string();
    valid_slug(&s).then_some(s)
}

// ------------------------------------------------------------------ parsed pages

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub slug: String,
    pub title: String,
    pub thumb: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SearchPage {
    pub hits: Vec<Hit>,
    /// 0-based page shown.
    pub page: u32,
    /// 0-based index of the last page.
    pub last_page: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// Medium-size image for the store.
    pub image: String,
    /// What it links to (full image, or an audio file).
    pub full: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileLink {
    pub url: String,
    /// The original file name (`download` attribute), else the URL's last segment.
    pub name: String,
    pub mime: Option<String>,
    /// Exact size in bytes when the page says it (`type="…; length=N"`).
    pub size: Option<u64>,
}

impl FileLink {
    pub fn ext(&self) -> String {
        std::path::Path::new(&self.name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    }

    pub fn is_zip(&self) -> bool {
        self.ext() == "zip" || self.mime.as_deref() == Some("application/zip")
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Asset {
    pub slug: String,
    pub title: String,
    pub author: String,
    pub author_url: Option<String>,
    pub collaborators: Vec<String>,
    pub date: String,
    pub art_types: Vec<String>,
    pub tags: Vec<String>,
    pub licenses: Vec<License>,
    pub description: String,
    /// "Copyright/Attribution Notice" written by the author.
    pub notice: String,
    pub previews: Vec<Preview>,
    pub files: Vec<FileLink>,
}

impl Asset {
    pub fn url(&self) -> String {
        content_url(&self.slug)
    }
}

// ------------------------------------------------------------------ HTML helpers

/// Decodes the entities OGA pages use (named basics and numeric ones).
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out += &rest[..i];
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace()).map(|j| j + 1);
        let Some(end) = end.filter(|&e| e <= 12 && rest.as_bytes().get(e) == Some(&b';')) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "copy" => Some('©'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "middot" => Some('·'),
            "times" => Some('×'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

/// Removes `<tag …>…</tag>` blocks (scripts, styles).
fn drop_blocks(html: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(i) = lower.find(&open) else { break };
        out += &rest[..i];
        match lower[i..].find(&close) {
            Some(j) => rest = &rest[i + j + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out + rest
}

/// HTML → readable plain text: paragraphs and line breaks kept, list items as `• `, tags
/// dropped, entities decoded, runs of spaces and blank lines collapsed.
pub fn plain_text(html: &str) -> String {
    let html = drop_blocks(&drop_blocks(html, "script"), "style");
    let mut text = String::new();
    let mut rest = html.as_str();
    while let Some(i) = rest.find('<') {
        text += &rest[..i];
        let Some(j) = rest[i..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[i + 1..i + j].trim().to_ascii_lowercase();
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        match name.as_str() {
            "br" | "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol" => text.push('\n'),
            "li" if !tag.starts_with('/') => text += "\n• ",
            _ => {}
        }
        rest = &rest[i + j + 1..];
    }
    text += rest;
    let text = decode_entities(&text);
    let mut lines: Vec<String> = vec![];
    for l in text.lines() {
        let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
        // compact: one line per paragraph / item, no blank lines
        if !l.is_empty() {
            lines.push(l);
        }
    }
    lines.join("\n")
}

/// Value of attribute `name` in the start tag `tag` (`<a href="…" …>`), entities decoded.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| i + from) {
        from = i + name.len();
        let before_ok = i > 0 && lower.as_bytes()[i - 1].is_ascii_whitespace();
        let after = lower[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let v = tag[tag.len() - after.len() + 1..].trim_start();
        let q = v.chars().next()?;
        let val = if q == '"' || q == '\'' {
            let end = v[1..].find(q)?;
            &v[1..1 + end]
        } else {
            v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("")
        };
        return Some(decode_entities(val));
    }
    None
}

/// Every `<name …>` start tag in `html` (the tag text, without inner HTML).
pub fn start_tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = lower[from..].find(&open).map(|i| i + from) {
        let after = lower.as_bytes().get(i + open.len()).copied().unwrap_or(b'>');
        from = i + open.len();
        if !(after.is_ascii_whitespace() || after == b'>' || after == b'/') {
            continue;
        }
        let Some(j) = html[i..].find('>') else { break };
        out.push(&html[i..i + j + 1]);
        from = i + j + 1;
    }
    out
}

/// `(start tag, inner HTML)` of every `<a>` in `html`.
pub fn links(html: &str) -> Vec<(&str, &str)> {
    let lower = html.to_ascii_lowercase();
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = lower[from..].find("<a ").map(|i| i + from) {
        let Some(j) = html[i..].find('>') else { break };
        let inner_from = i + j + 1;
        let end = lower[inner_from..].find("</a>").map(|k| k + inner_from).unwrap_or(html.len());
        out.push((&html[i..inner_from], &html[inner_from..end]));
        from = end.max(inner_from);
    }
    out
}

/// The HTML of Drupal field `field-name-<name>`, up to the next field (first occurrence).
pub fn field<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("field-name-{name}");
    let mut from = 0;
    loop {
        let i = html[from..].find(&key)? + from;
        let next = html.as_bytes().get(i + key.len()).copied().unwrap_or(b' ');
        from = i + key.len();
        if next == b' ' || next == b'"' || next == b'\'' {
            let start = html[from..].find('>').map(|j| from + j + 1)?;
            let end = html[start..].find("field field-name-").map(|j| start + j).unwrap_or(html.len());
            // back up to the start of that next `<div class="field …`
            let end = html[..end].rfind('<').filter(|&k| k >= start).unwrap_or(end);
            return Some(&html[start..end]);
        }
    }
}

/// Inner HTML of every `field-item` of a field (the label excluded).
pub fn field_items(seg: &str) -> Vec<&str> {
    let mut out = vec![];
    let starts: Vec<usize> = seg.match_indices("class=\"field-item ").map(|(i, _)| i).collect();
    for (n, &i) in starts.iter().enumerate() {
        let Some(open_end) = seg[i..].find('>').map(|j| i + j + 1) else { continue };
        let end = starts.get(n + 1).map(|&e| seg[..e].rfind('<').unwrap_or(e)).unwrap_or(seg.len());
        if end > open_end {
            out.push(&seg[open_end..end]);
        }
    }
    out
}

fn absolute(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else if url.starts_with('/') {
        format!("{BASE}{url}")
    } else {
        url.to_string()
    }
}

// ------------------------------------------------------------------ the two page parsers

/// A results page of `/art-search-advanced`.
pub fn parse_search(html: &str) -> Result<SearchPage, String> {
    let mut page = SearchPage::default();
    let mut seen = BTreeSet::new();
    let marks: Vec<usize> = html.match_indices("art-preview-title").map(|(i, _)| i).collect();
    for (n, &i) in marks.iter().enumerate() {
        let end = marks.get(n + 1).copied().unwrap_or(html.len());
        let block = &html[i..end];
        let Some((tag, inner)) = links(block).into_iter().next() else { continue };
        let Some(slug) = attr(tag, "href").as_deref().and_then(slug_of) else { continue };
        if !seen.insert(slug.clone()) {
            continue;
        }
        let thumb = start_tags(block, "img").into_iter().find_map(|t| attr(t, "src")).map(|s| absolute(&s)).filter(|u| allowed_url(u));
        page.hits.push(Hit { slug, title: plain_text(inner), thumb });
    }
    // pager: "pager-current" holds the 1-based page; "pager-last" links to `page=N` (0-based)
    if let Some(i) = html.find("pager-current") {
        let num: String = html[i..].split('>').nth(1).unwrap_or("").chars().take_while(|c| c.is_ascii_digit()).collect();
        page.page = num.parse::<u32>().unwrap_or(1).saturating_sub(1);
    }
    let page_of = |seg: &str| -> Option<u32> {
        let (tag, _) = links(seg).into_iter().next()?;
        let href = attr(tag, "href")?;
        href.split(['?', '&']).find_map(|kv| kv.strip_prefix("page=")).and_then(|v| v.parse().ok())
    };
    page.last_page = page.page;
    if let Some(i) = html.find("pager-last") {
        if let Some(p) = page_of(&html[i..]) {
            page.last_page = p.max(page.page);
        }
    } else if let Some(i) = html.find("pager-item") {
        // no "last" link: the highest numbered page link
        let pager = &html[i..html[i..].find("</ul>").map(|j| i + j).unwrap_or(html.len())];
        for (tag, _) in links(pager) {
            if let Some(p) = attr(tag, "href").and_then(|h| h.split(['?', '&']).find_map(|kv| kv.strip_prefix("page=").and_then(|v| v.parse::<u32>().ok()))) {
                page.last_page = page.last_page.max(p);
            }
        }
    }
    if page.hits.is_empty() && !html.contains("art-search-advanced") && !html.contains("view-empty") {
        return Err("la respuesta de OpenGameArt no tiene el formato esperado (¿ha cambiado la web?)".into());
    }
    Ok(page)
}

/// An asset page `/content/<slug>`.
pub fn parse_detail(html: &str, slug: &str) -> Result<Asset, String> {
    // the node ends where the comments start: their usernames and dates are not the asset's
    let html = match html.find("id=\"comments\"") {
        Some(i) => &html[..i],
        None => html,
    };
    let title = field(html, "title").map(plain_text).unwrap_or_default();
    if title.is_empty() || !html.contains("field-name-field-art-") {
        return Err("no parece la página de un asset de OpenGameArt (¿ha cambiado la web?)".into());
    }
    let mut a = Asset { slug: slug.to_string(), title, ..Default::default() };
    if let Some(seg) = field(html, "author-submitter") {
        let items = field_items(seg);
        let item = items.first().copied().unwrap_or(seg);
        match links(item).into_iter().next() {
            Some((tag, inner)) => {
                a.author = plain_text(inner);
                a.author_url = attr(tag, "href").map(|h| absolute(&h));
            }
            None => a.author = plain_text(item),
        }
    }
    let list = |name: &str| -> Vec<String> { field(html, name).map(|s| field_items(s).into_iter().map(plain_text).filter(|t| !t.is_empty()).collect()).unwrap_or_default() };
    a.collaborators = list("field-collaborators");
    a.date = list("post-date").into_iter().next().unwrap_or_default();
    a.art_types = list("field-art-type");
    a.tags = list("field-art-tags");
    if let Some(seg) = field(html, "field-art-licenses") {
        for item in field_items(seg) {
            // the name is in <div class='license-name'>…</div>; fall back to the item text
            let name = item.find("license-name").and_then(|i| item[i..].split_once('>')).map(|(_, r)| plain_text(r.split('<').next().unwrap_or(""))).filter(|s| !s.is_empty()).unwrap_or_else(|| plain_text(item));
            if !name.is_empty() {
                let l = License::parse(&name);
                if !a.licenses.contains(&l) {
                    a.licenses.push(l);
                }
            }
        }
    }
    a.description = field(html, "body").map(plain_text).unwrap_or_default();
    a.notice = field(html, "field-copyright-notice").map(|s| field_items(s).into_iter().map(plain_text).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    if let Some(seg) = field(html, "field-art-preview") {
        for item in field_items(seg) {
            let full = links(item).into_iter().next().and_then(|(t, _)| attr(t, "href")).map(|h| absolute(&h));
            if let Some(src) = start_tags(item, "img").into_iter().find_map(|t| attr(t, "src")) {
                a.previews.push(Preview { image: absolute(&src), full });
            } else if let Some(f) = full {
                // audio previews: a link to the file, no image
                a.previews.push(Preview { image: String::new(), full: Some(f) });
            }
        }
    }
    if let Some(seg) = field(html, "field-art-files") {
        for (tag, inner) in links(seg) {
            let Some(url) = attr(tag, "href").map(|h| absolute(&h)) else { continue };
            if !url.starts_with(&format!("{BASE}/sites/default/files/")) {
                continue;
            }
            let ty = attr(tag, "type").unwrap_or_default();
            let mut parts = ty.split(';').map(str::trim);
            let mime = parts.next().filter(|m| !m.is_empty()).map(String::from);
            let size = parts.find_map(|p| p.strip_prefix("length=")).and_then(|n| n.parse().ok());
            let name = attr(tag, "download")
                .filter(|n| !n.trim().is_empty())
                .or_else(|| Some(plain_text(inner)).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| modengine::import::percent_decode(url.rsplit('/').next().unwrap_or("file")));
            a.files.push(FileLink { url, name, mime, size });
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fixture(name: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oga").join(name)).unwrap()
    }

    #[test]
    fn search_urls() {
        let q = Query { keys: "slime  ".into(), kind: ArtKind::Art2D, licenses: [License::Cc0].into_iter().collect(), sort: Sort::Favorites, page: 0 };
        assert_eq!(q.url(), "https://opengameart.org/art-search-advanced?keys=slime&field_art_type_tid%5B%5D=9&field_art_licenses_tid%5B%5D=4&sort_by=count&sort_order=DESC&page=0");
        let q = Query { keys: "pixel font ñ&".into(), kind: ArtKind::Font, licenses: License::default_filter(), sort: Sort::Newest, page: 3 };
        let u = q.url();
        assert!(u.starts_with("https://opengameart.org/art-search-advanced?keys=pixel+font+%C3%B1%26&field_art_tags_tid=font"), "{u}");
        for t in [4, 2, 17981, 10310, 31772] {
            assert!(u.contains(&format!("field_art_licenses_tid%5B%5D={t}&")), "{u}");
        }
        assert!(!u.contains("field_art_type_tid") && u.ends_with("sort_by=created&sort_order=DESC&page=3"));
        assert_eq!(Query { kind: ArtKind::Music, ..Default::default() }.url().matches("field_art_type_tid%5B%5D=12").count(), 1);
        assert!(allowed_url(&q.url()));
    }

    #[test]
    fn only_allowed_paths() {
        assert!(allowed_url("https://opengameart.org/content/animated-slime"));
        assert!(allowed_url("https://opengameart.org/sites/default/files/styles/thumbnail/public/a.png"));
        assert!(!allowed_url("https://opengameart.org/search/node/slime"), "robots.txt: Disallow /search/");
        assert!(!allowed_url("https://opengameart.org/users/calciumtrice"));
        assert!(!allowed_url("https://evil.example/content/x"));
        assert!(!allowed_url("http://opengameart.org/content/x"));
        assert!(!allowed_url("https://opengameart.org.evil/content/x"));
        assert!(!allowed_url("https://opengameart.org/content/../user/login"));
        assert_eq!(slug_of("/content/a-blocky-dungeon"), Some("a-blocky-dungeon".into()));
        assert_eq!(slug_of("https://opengameart.org/content/x-1#comments"), Some("x-1".into()));
        assert_eq!(slug_of("/forumtopic/x"), None);
        assert_eq!(slug_of("/content/../x"), None);
    }

    #[test]
    fn licenses_parse_and_warn() {
        assert_eq!(License::parse("CC0"), License::Cc0);
        assert_eq!(License::parse(" CC-BY 3.0 "), License::CcBy3);
        assert_eq!(License::parse("CC-BY-SA 4.0"), License::CcBySa4);
        assert_eq!(License::parse("OGA-BY 3.0"), License::OgaBy3);
        assert_eq!(License::parse("GPL 2.0"), License::Gpl2);
        assert_eq!(License::parse("LGPL 3.0"), License::Lgpl3);
        assert_eq!(License::parse("CC-BY 2.5"), License::Other("CC-BY 2.5".into()));
        for l in License::FILTERS {
            assert_eq!(License::parse(&l.name()), l);
            assert_eq!(License::from_key(&l.key()), Some(l.clone()));
            assert!(l.tid().is_some() && l.url().is_some());
        }
        let has = |ls: &[License], n: Note, s: &str| license_notes(ls).iter().any(|(k, t)| *k == n && t.contains(s));
        assert!(has(&[License::Cc0], Note::Info, "no exige crédito"));
        assert!(blocking_note(&[License::Cc0]).is_none());
        assert!(has(&[License::CcBy3], Note::Warn, "crédito"));
        assert!(has(&[License::CcBySa3], Note::Warn, "misma licencia"));
        assert!(has(&[License::Gpl3], Note::Warn, "GPL"));
        assert!(has(&[License::OgaBy3], Note::Info, "DRM"));
        // CC0 or SA: the user may pick CC0, nothing forced
        assert!(!has(&[License::Cc0, License::CcBySa3], Note::Warn, "misma licencia") && has(&[License::Cc0, License::CcBySa3], Note::Info, "varias"));
        assert!(blocking_note(&[]).is_some());
        assert!(blocking_note(&[License::Other("Custom".into())]).unwrap().contains("Custom"));
        assert!(blocking_note(&[License::Other("x".into()), License::CcBy4]).is_none());
        assert_eq!((License::Cc0.level(), License::CcBy4.level(), License::Gpl2.level(), License::Other("x".into()).level()), (0, 1, 2, 3));
    }

    #[test]
    fn html_helpers() {
        assert_eq!(decode_entities("a &amp; b &#039;c&#39; &lt;d&gt; &#x41;&nbsp;&bogus; & x"), "a & b 'c' <d> A &bogus; & x");
        assert_eq!(plain_text("<p>Hola <b>mundo</b></p>\n<p>-idle</p><ul><li>a</li><li>b</li></ul><script>x()</script>"), "Hola mundo\n-idle\n• a\n• b");
        assert_eq!(attr("<a href=\"/x?a=1&amp;b=2\" data-href='no' title='T'>", "href").as_deref(), Some("/x?a=1&b=2"));
        assert_eq!(attr("<img data-src='no' src='y.png'>", "src").as_deref(), Some("y.png"));
        assert_eq!(attr("<a class=x>", "href"), None);
        assert_eq!(links("<a href='1'>uno</a> <abbr>no</abbr><a href=\"2\"><b>dos</b></a>").iter().map(|(t, i)| (attr(t, "href").unwrap(), plain_text(i))).collect::<Vec<_>>(), vec![("1".into(), "uno".into()), ("2".into(), "dos".into())]);
    }

    #[test]
    fn parses_a_real_results_page() {
        let p = parse_search(&fixture("search_slime_cc0.html")).unwrap();
        assert_eq!(p.hits.len(), 24);
        assert_eq!(p.hits[0], Hit { slug: "a-blocky-dungeon".into(), title: "A blocky dungeon".into(), thumb: p.hits[0].thumb.clone() });
        assert!(p.hits[0].thumb.as_deref().unwrap().starts_with("https://opengameart.org/sites/default/files/styles/thumbnail/public/"));
        let slime = p.hits.iter().find(|h| h.slug == "pixel-art-animated-slime").unwrap();
        assert_eq!(slime.title, "Pixel art animated Slime");
        assert!(p.hits.iter().any(|h| h.title == "32x32 colorful slimes!"));
        assert!(p.hits.iter().all(|h| h.thumb.is_some()));
        assert_eq!((p.page, p.last_page), (0, 3));
        // an empty result page is not an error; garbage is
        assert_eq!(parse_search("<html><form action=\"/art-search-advanced\"></form></html>").unwrap().hits.len(), 0);
        assert!(parse_search("<html>Service unavailable</html>").is_err());
    }

    #[test]
    fn parses_real_asset_pages() {
        let a = parse_detail(&fixture("animated-slime.html"), "animated-slime").unwrap();
        assert_eq!(a.title, "Animated Slime");
        assert_eq!(a.author, "Calciumtrice");
        assert_eq!(a.author_url.as_deref(), Some("https://opengameart.org/users/calciumtrice"));
        assert_eq!(a.date, "Sunday, May 24, 2015 - 07:40", "the node's date, not a comment's");
        assert_eq!(a.art_types, vec!["2D Art"]);
        assert_eq!(a.tags, vec!["Pixel Art", "monster", "slime"]);
        assert_eq!(a.licenses, vec![License::CcBy3]);
        assert!(a.description.starts_with("Animated Slime by Calciumtrice, usable under Creative Commons Attribution 3.0 license."), "{}", a.description);
        assert_eq!(a.notice, "Animated Slime by Calciumtrice, usable under Creative Commons Attribution 3.0 license.");
        assert_eq!(a.previews.len(), 4);
        assert_eq!(a.previews[0].image, "https://opengameart.org/sites/default/files/styles/medium/public/slime_idle_0.gif");
        assert_eq!(a.previews[0].full.as_deref(), Some("https://opengameart.org/sites/default/files/slime_idle_0.gif"));
        assert_eq!(
            a.files,
            vec![FileLink {
                url: "https://opengameart.org/sites/default/files/slime%20spritesheet%20calciumtrice_0.png".into(),
                name: "slime spritesheet calciumtrice.png".into(),
                mime: Some("image/png".into()),
                size: Some(12041),
            }]
        );
        assert!(!a.files[0].is_zip() && a.files[0].ext() == "png");

        let z = parse_detail(&fixture("pixel-art-animated-slime.html"), "pixel-art-animated-slime").unwrap();
        assert_eq!((z.author.as_str(), z.licenses.as_slice()), ("rvros", &[License::Cc0][..]));
        assert_eq!(z.description, "Platformer slime with animations including:\n-idle\n-move,\n-attack,\n-hurt");
        assert_eq!(z.files.len(), 1);
        assert!(z.files[0].is_zip() && z.files[0].name == "Slime.zip" && z.files[0].size == Some(12061));

        let t = parse_detail(&fixture("tiny-creatures.html"), "tiny-creatures").unwrap();
        assert_eq!(t.collaborators, vec!["Kenney"]);
        assert_eq!(t.licenses, vec![License::Cc0]);
        assert!(t.files[0].is_zip());
        assert!(!t.description.contains("<"), "plain text");

        assert!(parse_detail("<html><body>Access denied</body></html>", "x").is_err());
    }
}
