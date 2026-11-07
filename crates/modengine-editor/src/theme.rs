//! Editor look: a light theme modelled on the Hermes Agent Desktop light skin, plus the
//! classic egui dark theme.
//!
//! Hermes defines its light theme with a few seed colours (`--theme-*`) and derives every
//! surface, stroke and fill with CSS `color-mix()`. The values below are those mixes
//! resolved to sRGB over the surface they are drawn on, so the editor matches it without
//! needing a CSS engine:
//!
//! | Hermes variable            | seed / mix                                       | here            |
//! |----------------------------|--------------------------------------------------|-----------------|
//! | `--ui-bg-chrome`           | `#f8faff` 92% + `#f3f3f3`                         | `CHROME`        |
//! | `--ui-bg-sidebar`          | `#f3f7ff`                                         | `SIDEBAR`       |
//! | `--ui-bg-editor`           | `#fff` 22% + `#fcfcfc`                            | `CARD`          |
//! | `--ui-base`                | `#17171a` (text = base at 94/74/54/36 %)          | `TEXT*`         |
//! | `--ui-accent`              | `#0053fd` (midground)                             | `ACCENT`        |
//! | `--ui-stroke-*`            | accent 24/16/10/6 % over base 10/7/5/3 %          | `STROKE*`       |
//! | `--ui-row-hover/active`    | accent 4/8 % over base 3/5 %                      | `ROW_*`         |
//! | `--radius-md`              | `.75rem × .2` ≈ 2–3 px                            | `RADIUS`        |
//! | code blocks                | Shiki `github-light-default`                      | `code_palette`  |

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

// ---- Hermes light, resolved
pub const CHROME: Color32 = Color32::from_rgb(248, 249, 254);
pub const SIDEBAR: Color32 = Color32::from_rgb(243, 247, 255);
pub const CARD: Color32 = Color32::from_rgb(253, 253, 253);
pub const INPUT: Color32 = Color32::from_rgb(252, 252, 252);
pub const ACCENT: Color32 = Color32::from_rgb(0, 83, 253);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(230, 238, 255);
pub const TEXT: Color32 = Color32::from_rgb(36, 37, 40);
pub const TEXT2: Color32 = Color32::from_rgb(81, 82, 85);
pub const TEXT3: Color32 = Color32::from_rgb(126, 127, 131);
#[allow(dead_code)]
pub const TEXT4: Color32 = Color32::from_rgb(167, 168, 172);
pub const STROKE1: Color32 = Color32::from_rgb(171, 192, 236);
pub const STROKE2: Color32 = Color32::from_rgb(195, 209, 240);
pub const STROKE3: Color32 = Color32::from_rgb(213, 223, 244);
#[allow(dead_code)]
pub const STROKE4: Color32 = Color32::from_rgb(226, 233, 248);
pub const ROW_HOVER: Color32 = Color32::from_rgb(231, 236, 247);
pub const ROW_ACTIVE: Color32 = Color32::from_rgb(217, 226, 243);
pub const RED: Color32 = Color32::from_rgb(0xcf, 0x2d, 0x56);
pub const GREEN: Color32 = Color32::from_rgb(0x1f, 0x8a, 0x65);
pub const YELLOW: Color32 = Color32::from_rgb(0xc0, 0x85, 0x32);
pub const BLUE: Color32 = Color32::from_rgb(0x00, 0x53, 0xfd);

pub const RADIUS: u8 = 3;

/// Colours the editors use for their own drawing (logs, canvases, overlays).
#[derive(Clone, Copy)]
pub struct Colors {
    pub ok: Color32,
    pub error: Color32,
    pub info: Color32,
    pub game: Color32,
    pub weak: Color32,
    /// Empty area around a room or sprite canvas.
    pub canvas: Color32,
    pub checker_a: Color32,
    pub checker_b: Color32,
    pub canvas_border: Color32,
    pub canvas_text: Color32,
    pub selection: Color32,
    pub camera: Color32,
    pub frame_border: Color32,
    /// Workspace canvas, its dot grid, chain links and window title bars.
    pub workspace: Color32,
    pub workspace_dot: Color32,
    pub chain: Color32,
    pub title: Color32,
    pub title_active: Color32,
}

pub fn colors(ctx: &egui::Context) -> Colors {
    if ctx.global_style().visuals.dark_mode {
        Colors {
            ok: Color32::from_rgb(130, 220, 130),
            error: Color32::from_rgb(255, 120, 120),
            info: Color32::from_gray(190),
            game: Color32::from_rgb(170, 200, 255),
            weak: Color32::from_gray(150),
            canvas: Color32::from_gray(24),
            checker_a: Color32::from_gray(58),
            checker_b: Color32::from_gray(72),
            canvas_border: Color32::from_gray(160),
            canvas_text: Color32::from_gray(170),
            selection: Color32::from_rgb(255, 200, 80),
            camera: Color32::from_rgb(90, 200, 255),
            frame_border: Color32::from_gray(90),
            workspace: Color32::from_rgb(22, 24, 28),
            workspace_dot: Color32::from_rgb(48, 52, 60),
            chain: Color32::from_rgb(90, 140, 220),
            title: Color32::from_rgb(40, 41, 46),
            title_active: Color32::from_rgb(44, 56, 80),
        }
    } else {
        Colors {
            ok: GREEN,
            error: RED,
            info: TEXT2,
            game: BLUE,
            weak: TEXT3,
            canvas: Color32::from_rgb(234, 238, 247),
            checker_a: Color32::from_rgb(236, 238, 243),
            checker_b: Color32::from_rgb(222, 226, 235),
            canvas_border: STROKE1,
            canvas_text: TEXT3,
            selection: Color32::from_rgb(255, 160, 0),
            camera: ACCENT,
            frame_border: STROKE2,
            workspace: Color32::from_rgb(236, 240, 249),
            workspace_dot: Color32::from_rgb(206, 215, 236),
            chain: Color32::from_rgb(120, 155, 235),
            title: Color32::from_rgb(243, 246, 252),
            title_active: ACCENT_SOFT,
        }
    }
}

/// Syntax colours for ModScript.
pub struct CodePalette {
    pub text: Color32,
    pub keyword: Color32,
    pub value: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub builtin: Color32,
    pub var: Color32,
}

pub fn code_palette(dark: bool) -> CodePalette {
    if dark {
        CodePalette {
            text: Color32::from_rgb(212, 216, 224),
            keyword: Color32::from_rgb(232, 140, 200),
            value: Color32::from_rgb(240, 170, 110),
            string: Color32::from_rgb(160, 210, 120),
            number: Color32::from_rgb(130, 200, 240),
            comment: Color32::from_rgb(120, 130, 145),
            builtin: Color32::from_rgb(110, 190, 255),
            var: Color32::from_rgb(250, 220, 130),
        }
    } else {
        // GitHub Light Default (what Hermes' Shiki renderer uses in light mode).
        CodePalette {
            text: Color32::from_rgb(0x1f, 0x23, 0x28),
            keyword: Color32::from_rgb(0xcf, 0x22, 0x2e),
            value: Color32::from_rgb(0x05, 0x50, 0xae),
            string: Color32::from_rgb(0x0a, 0x30, 0x69),
            number: Color32::from_rgb(0x05, 0x50, 0xae),
            comment: Color32::from_rgb(0x59, 0x63, 0x6e),
            builtin: Color32::from_rgb(0x82, 0x50, 0xdf),
            var: Color32::from_rgb(0x95, 0x38, 0x00),
        }
    }
}

fn light_visuals() -> Visuals {
    let mut v = Visuals::light();
    let r = CornerRadius::same(RADIUS);

    v.override_text_color = None;
    v.weak_text_color = Some(TEXT3);
    v.hyperlink_color = ACCENT;
    v.panel_fill = CHROME;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, STROKE2);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(5);
    // Hermes --shadow-md: 0 4px 16px #00000014, 0 16px 32px -24px #0000002e
    v.window_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(0x14) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(0x14) };
    v.faint_bg_color = Color32::from_rgb(244, 246, 252);
    v.extreme_bg_color = INPUT;
    v.code_bg_color = Color32::from_rgb(240, 242, 246);
    v.text_edit_bg_color = Some(INPUT);
    v.warn_fg_color = YELLOW;
    v.error_fg_color = RED;
    v.selection.bg_fill = Color32::from_rgb(204, 220, 255);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    v.striped = false;
    v.indent_has_left_vline = true;
    v.collapsing_header_frame = false;

    let w = &mut v.widgets;
    // labels, separators, frames
    w.noninteractive.bg_fill = CHROME;
    w.noninteractive.weak_bg_fill = CHROME;
    w.noninteractive.bg_stroke = Stroke::new(1.0, STROKE3);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = r;
    // buttons at rest: white "card" with a soft accent-tinted outline
    w.inactive.bg_fill = Color32::from_rgb(236, 240, 250);
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = Stroke::new(1.0, STROKE2);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    // hover = --ui-control-hover-background
    w.hovered.bg_fill = Color32::from_rgb(224, 231, 245);
    w.hovered.weak_bg_fill = ROW_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, STROKE1);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    // pressed = --ui-control-active-background
    w.active.bg_fill = ROW_ACTIVE;
    w.active.weak_bg_fill = ROW_ACTIVE;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    // open menus / combo boxes
    w.open.bg_fill = ACCENT_SOFT;
    w.open.weak_bg_fill = ACCENT_SOFT;
    w.open.bg_stroke = Stroke::new(1.0, STROKE1);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = r;
    v
}

fn dark_visuals() -> Visuals {
    Visuals::dark()
}

pub fn apply(ctx: &egui::Context, theme: Theme) {
    ctx.set_visuals_of(egui::Theme::Light, light_visuals());
    ctx.set_visuals_of(egui::Theme::Dark, dark_visuals());
    ctx.set_theme(match theme {
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
    ctx.all_styles_mut(|s| {
        // Hermes: 13px body, 11–12px captions, tight 2–3px radii, compact rows.
        s.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(10);
        s.spacing.interact_size.y = 22.0;
    });
}

/// Use Noto Sans (closest metric match to Segoe/SF that Linux usually ships) for the UI when
/// it's installed, otherwise egui's built-in font; and Font Awesome for the icons
/// ([`crate::icons`]): solid right after the text font in every family (before egui's own
/// icon fonts), the outlined style in its own family.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    ];
    if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("ui-sans".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui-sans".into());
    }
    let fa = |bytes: &'static [u8]| std::sync::Arc::new(egui::FontData::from_static(bytes).tweak(egui::FontTweak { scale: 0.9, ..Default::default() }));
    fonts.font_data.insert("fa-solid".into(), fa(include_bytes!("../assets/fonts/fa-solid-900.ttf")));
    fonts.font_data.insert(crate::icons::REGULAR.into(), fa(include_bytes!("../assets/fonts/fa-regular-400.ttf")));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(1.min(list.len()), "fa-solid".into());
    }
    let mut regular = vec![crate::icons::REGULAR.to_string()];
    regular.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Name(crate::icons::REGULAR.into()), regular);
    ctx.set_fonts(fonts);
}

fn theme_file() -> Option<std::path::PathBuf> {
    modengine::home_dir().map(|h| h.join(".config/modengine/theme.txt"))
}

pub fn load() -> Theme {
    match theme_file().and_then(|f| std::fs::read_to_string(f).ok()).as_deref().map(str::trim) {
        Some("dark") => Theme::Dark,
        _ => Theme::Light,
    }
}

pub fn save(t: Theme) {
    if let Some(f) = theme_file() {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, if t == Theme::Dark { "dark" } else { "light" });
    }
}

/// Side panel frame tinted like Hermes' sidebar.
pub fn sidebar_frame(ctx: &egui::Context) -> egui::Frame {
    let s = ctx.global_style();
    let mut f = egui::Frame::side_top_panel(&s);
    if !s.visuals.dark_mode {
        f.fill = SIDEBAR;
        f.stroke = Stroke::new(1.0, STROKE2);
    }
    f
}

/// Central / editor area frame: the white "card" surface.
pub fn editor_frame(ctx: &egui::Context) -> egui::Frame {
    let s = ctx.global_style();
    let mut f = egui::Frame::central_panel(&s);
    if !s.visuals.dark_mode {
        f.fill = CARD;
    }
    f
}

/// Filled accent button (Hermes `--dt-primary` with `--dt-primary-foreground`).
pub fn primary_button(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    if ui.visuals().dark_mode {
        return egui::Button::new(text.to_string());
    }
    egui::Button::new(egui::RichText::new(text.to_string()).color(Color32::from_rgb(0xfc, 0xfc, 0xfc)))
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS))
}
