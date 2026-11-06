//! Drawing sprites fast and well: every small sprite sheet is packed into a few big atlas
//! textures (so thousands of instances with different sprites stay in one draw call, in
//! exactly the order of the draw list), and sprites with `rotation = "rotsprite"` draw from
//! cached RotSprite images, one per (frame, angle step).
//!
//! While a custom shader is active sprites draw from their own texture, as before atlases,
//! so shaders that read `v_vTexcoord` see the coordinates of the sprite sheet.

use macroquad::prelude::*;
use modengine::project::Sprite;
use modengine::rotsprite::{self, Img, RotMode};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

static LAST_QUADS: AtomicU64 = AtomicU64::new(0);
static LAST_SWITCHES: AtomicU64 = AtomicU64::new(0);

/// Sprites drawn and texture changes between them in the last rendered frame (each
/// change starts a new GPU draw call).
pub fn last_frame_stats() -> (u64, u64) {
    (LAST_QUADS.load(Ordering::Relaxed), LAST_SWITCHES.load(Ordering::Relaxed))
}

#[cfg(target_os = "android")]
const PAGE: u32 = 2048;
#[cfg(not(target_os = "android"))]
const PAGE: u32 = 4096;
/// Sheets bigger than this keep their own texture.
const MAX_PACKED: u32 = 1024;
/// Edge pixels copied around each sheet so filtering never reads a neighbour.
const PAD: u32 = 2;
/// RotSprite images kept on the GPU (bytes) before the least recently used go.
const CACHE_BYTES: usize = 96 << 20;
/// Milliseconds per frame spent computing new RotSprite angles; the rest draw with the
/// GPU rotation this frame and are computed in the next ones.
const BUDGET_MS: f64 = 6.0;

struct Rot {
    tex: Texture2D,
    ox: f32,
    oy: f32,
    w: f32,
    h: f32,
    used: u64,
    bytes: usize,
}

pub struct SpriteDraw {
    pages: Vec<Texture2D>,
    /// sprite → (atlas page, x, y of its sheet inside the page)
    place: Vec<Option<(usize, f32, f32)>>,
    /// 8× Scale2x images of frames being rotated, by (sprite, frame)
    big: HashMap<(u32, u32), Img>,
    rots: HashMap<(u32, u32, u32), Rot>,
    bytes: usize,
    frame_no: u64,
    spent: f64,
    /// Linear filtering for RotSprite textures (`smooth` games).
    filter: FilterMode,
    /// Sprites drawn and texture changes so far this frame.
    stats: (u64, u64),
    last_tex: Option<macroquad::miniquad::TextureId>,
    /// Overwrite colour + amount of the following sprites (`DrawCmd::SpriteFx`).
    fx: Option<[f32; 4]>,
}

/// The default material of the game (see `gpu_fx::compile`): GameMaker's pass-through
/// shader plus the sprite overwrite colour, which arrives in the vertex `normal`
/// (rgb + amount; zero for every other draw, so they look exactly as before).
pub const FX_VSH: &str = "\
attribute vec3 in_Position;
attribute vec4 in_Colour;
attribute vec2 in_TextureCoord;
varying vec2 v_vTexcoord;
varying vec4 v_vColour;
varying vec4 v_vFx;
void main()
{
    gl_Position = gm_Matrices[MATRIX_WORLD_VIEW_PROJECTION] * vec4(in_Position, 1.0);
    v_vColour = in_Colour;
    v_vTexcoord = in_TextureCoord;
    v_vFx = normal;
}
";

pub const FX_FSH: &str = "\
varying vec2 v_vTexcoord;
varying vec4 v_vColour;
varying vec4 v_vFx;
void main()
{
    vec4 c = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord);
    gl_FragColor = vec4(mix(c.rgb, v_vFx.rgb, v_vFx.a), c.a);
}
";

/// `draw_texture_ex` with the overwrite colour in the vertices' `normal`.
#[allow(clippy::too_many_arguments)]
fn quad_fx(tex: &Texture2D, x: f32, y: f32, color: Color, src: Rect, dest: Vec2, rotation: f32, flip_x: bool, flip_y: bool, pivot: Option<Vec2>, fx: [f32; 4]) {
    use macroquad::models::Vertex;
    let (tw, th) = (tex.width(), tex.height());
    let (mut x, mut y, mut w, mut h) = (x, y, dest.x, dest.y);
    if flip_x {
        x += w;
        w = -w;
    }
    if flip_y {
        y += h;
        h = -h;
    }
    let pivot = pivot.unwrap_or(vec2(x + w / 2.0, y + h / 2.0));
    let (s, c) = rotation.sin_cos();
    let rot = |p: Vec2| {
        let d = p - pivot;
        vec2(d.x * c - d.y * s, d.x * s + d.y * c) + pivot
    };
    let p = [rot(vec2(x, y)), rot(vec2(x + w, y)), rot(vec2(x + w, y + h)), rot(vec2(x, y + h))];
    let (u0, v0, u1, v1) = (src.x / tw, src.y / th, (src.x + src.w) / tw, (src.y + src.h) / th);
    let n = Vec4::from_array(fx);
    let col: [u8; 4] = color.into();
    let vert = |p: Vec2, u: f32, v: f32| Vertex { position: vec3(p.x, p.y, 0.0), uv: vec2(u, v), color: col, normal: n };
    let verts = [vert(p[0], u0, v0), vert(p[1], u1, v0), vert(p[2], u1, v1), vert(p[3], u0, v1)];
    let gl = unsafe { get_internal_gl() }.quad_gl;
    gl.texture(Some(tex));
    gl.draw_mode(DrawMode::Triangles);
    gl.geometry(&verts, &[0, 1, 2, 0, 2, 3]);
}


impl SpriteDraw {
    /// Packs the sprites' sheets into atlas pages. Sprites drawn with linear filtering (a
    /// `smooth` game, `rotation = "smooth"`, the particle shapes in `linear_range`) go to
    /// pages of their own; `textures` (each sprite's own texture) get the same filter.
    pub fn new(sprites: &[Sprite], textures: &[Texture2D], smooth: bool, linear_range: std::ops::Range<usize>) -> SpriteDraw {
        let linear = |i: usize| smooth || linear_range.contains(&i) || sprites[i].rotation.mode == RotMode::Smooth;
        for (i, t) in textures.iter().enumerate().take(sprites.len()) {
            if linear(i) {
                t.set_filter(FilterMode::Linear);
            }
        }
        let filter = if smooth { FilterMode::Linear } else { FilterMode::Nearest };
        let mut place = vec![None; sprites.len()];
        let mut pages = Vec::new();
        // MODENGINE_NO_ATLAS=1: every sprite draws from its own texture (diagnostics)
        let kinds: &[bool] = if std::env::var_os("MODENGINE_NO_ATLAS").is_some() { &[] } else { &[false, true] };
        for &want_linear in kinds {
            let ids: Vec<usize> = (0..sprites.len())
                .filter(|&i| linear(i) == want_linear && sprites[i].sheet_w > 0 && sprites[i].sheet_h > 0 && sprites[i].rgba.len() >= (sprites[i].sheet_w * sprites[i].sheet_h * 4) as usize)
                .collect();
            let sizes: Vec<(u32, u32)> = ids.iter().map(|&i| (sprites[i].sheet_w, sprites[i].sheet_h)).collect();
            let (spots, heights) = pack(&sizes, PAGE);
            let mut page_px: Vec<Vec<u8>> = heights.iter().map(|h| vec![0u8; (PAGE * h * 4) as usize]).collect();
            for (k, spot) in spots.iter().enumerate() {
                let Some((p, x, y)) = *spot else { continue };
                blit_padded(&mut page_px[p], &sprites[ids[k]], x, y);
                place[ids[k]] = Some((pages.len() + p, (x + PAD) as f32, (y + PAD) as f32));
            }
            for (px, h) in page_px.into_iter().zip(heights) {
                let t = Texture2D::from_rgba8(PAGE as u16, h as u16, &px);
                t.set_filter(if want_linear { FilterMode::Linear } else { FilterMode::Nearest });
                pages.push(t);
            }
        }
        SpriteDraw { pages, place, big: HashMap::new(), rots: HashMap::new(), bytes: 0, frame_no: 0, spent: 0.0, filter, stats: (0, 0), last_tex: None, fx: None }
    }

    /// `DrawCmd::SpriteFx`: overwrite colour of the following sprites (None = off).
    pub fn set_fx(&mut self, fx: Option<[f32; 4]>) {
        self.fx = fx.filter(|f| f[3] > 0.0);
    }

    /// Number of atlas pages (tests, stats).
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn begin_frame(&mut self) {
        LAST_QUADS.store(self.stats.0, Ordering::Relaxed);
        LAST_SWITCHES.store(self.stats.1, Ordering::Relaxed);
        self.frame_no += 1;
        self.spent = 0.0;
        self.stats = (0, 0);
        self.last_tex = None;
        self.fx = None;
    }

    fn count(&mut self, tex: &Texture2D) {
        let id = tex.raw_miniquad_id();
        self.stats.0 += 1;
        if self.last_tex != Some(id) {
            self.stats.1 += 1;
            self.last_tex = Some(id);
        }
    }

    /// Draws frame `frame` of sprite `si` like `draw_sprite_ext`: origin at (x, y), scale,
    /// angle in degrees counter-clockwise. `own` is the sprite's own texture; `shader` is
    /// true while a custom shader is active.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, sprites: &[Sprite], own: &Texture2D, si: u32, frame: u32, x: f32, y: f32, sx: f32, sy: f32, angle: f32, color: Color, shader: bool) {
        let Some(spr) = sprites.get(si as usize) else { return };
        let Some(fr) = spr.frames.get(frame as usize) else { return };
        let angle = spr.rotation.snap(angle);
        let quarter = (angle.rem_euclid(90.0)).abs() < 1e-3 || (90.0 - angle.rem_euclid(90.0)).abs() < 1e-3;
        if spr.rotation.mode == RotMode::Rotsprite && !quarter && (sx.abs() - sy.abs()).abs() < 1e-4 {
            if let Some(r) = self.rotated(spr, si, frame, fr, sx, sy, angle) {
                let (_, fx, fy) = rotsprite::plan(sx, sy, angle);
                let s = sx.abs();
                let left = if fx { x - (r.2 - r.0) * s } else { x - r.0 * s };
                let top = if fy { y - (r.3 - r.1) * s } else { y - r.1 * s };
                let tex = r.4.clone();
                self.count(&tex);
                if let (Some(sfx), false) = (self.fx, shader) {
                    let src = Rect::new(0.0, 0.0, tex.width(), tex.height());
                    quad_fx(&tex, left, top, color, src, vec2(r.2 * s, r.3 * s), 0.0, fx, fy, None, sfx);
                    return;
                }
                draw_texture_ex(&tex, left, top, color, DrawTextureParams { dest_size: Some(vec2(r.2 * s, r.3 * s)), flip_x: fx, flip_y: fy, ..Default::default() });
                return;
            }
        }
        let (tex, src_x, src_y) = match (shader, self.place.get(si as usize).copied().flatten()) {
            (false, Some((p, ax, ay))) => (self.pages[p].clone(), ax + fr[0] as f32, ay + fr[1] as f32),
            _ => (own.clone(), fr[0] as f32, fr[1] as f32),
        };
        self.count(&tex);
        let (fw, fh) = (fr[2] as f32, fr[3] as f32);
        let (ox, oy) = (spr.origin.0 as f32, spr.origin.1 as f32);
        let left = (x - ox * sx).min(x + (fw - ox) * sx);
        let top = (y - oy * sy).min(y + (fh - oy) * sy);
        if let (Some(sfx), false) = (self.fx, shader) {
            let dest = vec2(fw * sx.abs(), fh * sy.abs());
            quad_fx(&tex, left, top, color, Rect::new(src_x, src_y, fw, fh), dest, -angle.to_radians(), sx < 0.0, sy < 0.0, Some(vec2(x, y)), sfx);
            return;
        }
        draw_texture_ex(
            &tex,
            left,
            top,
            color,
            DrawTextureParams {
                dest_size: Some(vec2(fw * sx.abs(), fh * sy.abs())),
                source: Some(Rect::new(src_x, src_y, fw, fh)),
                rotation: -angle.to_radians(),
                flip_x: sx < 0.0,
                flip_y: sy < 0.0,
                pivot: Some(vec2(x, y)),
            },
        );
    }

    /// The cached RotSprite image for this draw: (origin x, origin y, w, h, texture).
    #[allow(clippy::too_many_arguments)]
    fn rotated(&mut self, spr: &Sprite, si: u32, frame: u32, fr: &[u32; 4], sx: f32, sy: f32, angle: f32) -> Option<(f32, f32, f32, f32, Texture2D)> {
        let (img_angle, _, _) = rotsprite::plan(sx, sy, angle);
        let (bucket, n) = spr.rotation.bucket(img_angle);
        let key = (si, frame, bucket);
        let now = self.frame_no;
        if let Some(r) = self.rots.get_mut(&key) {
            r.used = now;
            return Some((r.ox, r.oy, r.w, r.h, r.tex.clone()));
        }
        if self.spent > BUDGET_MS {
            return None;
        }
        let t0 = std::time::Instant::now();
        let src = Img::from_sheet(&spr.rgba, spr.sheet_w, *fr);
        let big = self.big.entry((si, frame)).or_insert_with(|| rotsprite::upscale8(&src));
        let a = bucket as f32 * 360.0 / n as f32;
        let r = rotsprite::rotate(&src, big, spr.origin.0 as f32, spr.origin.1 as f32, a);
        let tex = Texture2D::from_rgba8(r.img.w as u16, r.img.h as u16, &r.img.to_rgba());
        tex.set_filter(self.filter);
        let bytes = (r.img.w * r.img.h * 4) as usize;
        self.bytes += bytes;
        self.rots.insert(key, Rot { tex: tex.clone(), ox: r.ox, oy: r.oy, w: r.img.w as f32, h: r.img.h as f32, used: now, bytes });
        self.evict();
        self.spent += t0.elapsed().as_secs_f64() * 1000.0;
        Some((r.ox, r.oy, r.img.w as f32, r.img.h as f32, tex))
    }

    fn evict(&mut self) {
        while self.bytes > CACHE_BYTES && self.rots.len() > 1 {
            let Some((&k, _)) = self.rots.iter().min_by_key(|(_, r)| r.used) else { break };
            if let Some(r) = self.rots.remove(&k) {
                self.bytes -= r.bytes;
            }
        }
        // 8× images are big: keep the ones of frames rotated recently
        if self.big.len() > 64 {
            let live: std::collections::HashSet<(u32, u32)> = self.rots.iter().filter(|(_, r)| r.used + 120 > self.frame_no).map(|(k, _)| (k.0, k.1)).collect();
            self.big.retain(|k, _| live.contains(k));
        }
    }
}

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

/// Shelf packing of sheets (w, h) into `page`×`page` atlas pages, each with `PAD` pixels
/// around it: the spot (page, x, y) of every sheet (None = too big, keeps its own texture)
/// and the height of every page (a power of two: only as tall as what it holds).
pub fn pack(sizes: &[(u32, u32)], page: u32) -> (Vec<Option<(usize, u32, u32)>>, Vec<u32>) {
    let mut spots = vec![None; sizes.len()];
    let mut shelves: Vec<Vec<Shelf>> = vec![];
    // tallest first packs shelves tighter
    let mut order: Vec<usize> = (0..sizes.len()).filter(|&i| sizes[i].0 <= MAX_PACKED.min(page - PAD * 2) && sizes[i].1 <= MAX_PACKED.min(page - PAD * 2)).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i].1), i));
    for i in order {
        let (w, h) = (sizes[i].0 + PAD * 2, sizes[i].1 + PAD * 2);
        let mut spot = None;
        'pages: for (p, sh) in shelves.iter_mut().enumerate() {
            for shelf in sh.iter_mut() {
                if h <= shelf.h && shelf.x + w <= page {
                    spot = Some((p, shelf.x, shelf.y));
                    shelf.x += w;
                    break 'pages;
                }
            }
            let top = sh.last().map(|s| s.y + s.h).unwrap_or(0);
            if top + h <= page {
                sh.push(Shelf { y: top, h, x: w });
                spot = Some((p, 0, top));
                break;
            }
        }
        spots[i] = Some(spot.unwrap_or_else(|| {
            shelves.push(vec![Shelf { y: 0, h, x: w }]);
            (shelves.len() - 1, 0, 0)
        }));
    }
    let heights = shelves.iter().map(|sh| sh.last().map(|s| s.y + s.h).unwrap_or(1).next_power_of_two().min(page)).collect();
    (spots, heights)
}

/// Copies a sprite sheet into an atlas page at (x, y), with its edge pixels repeated
/// `PAD` times around it.
fn blit_padded(page: &mut [u8], s: &Sprite, x: u32, y: u32) {
    let (w, h) = (s.sheet_w, s.sheet_h);
    for py in 0..h + PAD * 2 {
        let sy = py.saturating_sub(PAD).min(h - 1);
        for px in 0..w + PAD * 2 {
            let sx = px.saturating_sub(PAD).min(w - 1);
            let si = ((sy * w + sx) * 4) as usize;
            let di = (((y + py) * PAGE + x + px) * 4) as usize;
            page[di..di + 4].copy_from_slice(&s.rgba[si..si + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_never_overlaps_and_skips_big_sheets() {
        let sizes: Vec<(u32, u32)> = (0..300).map(|i| (8 + (i * 37) % 120, 8 + (i * 53) % 90)).chain([(2000, 16), (64, 64)]).collect();
        let (spots, heights) = pack(&sizes, 512);
        assert!(spots[300].is_none(), "a 2000 px sheet keeps its own texture");
        assert!(heights.iter().all(|h| h.is_power_of_two() && *h <= 512));
        let mut rects: Vec<(usize, u32, u32, u32, u32)> = vec![];
        for (i, s) in spots.iter().enumerate() {
            let Some((p, x, y)) = *s else { continue };
            let (w, h) = (sizes[i].0 + PAD * 2, sizes[i].1 + PAD * 2);
            assert!(x + w <= 512 && y + h <= heights[p], "sheet {i} out of its page");
            for r in &rects {
                let apart = r.0 != p || x >= r.1 + r.3 || r.1 >= x + w || y >= r.2 + r.4 || r.2 >= y + h;
                assert!(apart, "sheet {i} overlaps another");
            }
            rects.push((p, x, y, w, h));
        }
        assert_eq!(rects.len(), 301);
        assert!(heights.len() < 20, "pages are reasonably full: {}", heights.len());
    }
}
