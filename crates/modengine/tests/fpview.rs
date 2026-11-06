//! First-person grid view (movement helpers, projection, picking, geometry), textured
//! primitives, the minimap and the pixelate post-process commands, run headless.

use modengine::draw::DrawCmd;
use modengine::fpview::{self as fp, FpView, MVert, PickKind};
use modengine::modscript::{intern, Value};
use modengine::Engine;
use std::path::{Path, PathBuf};

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A 5×3 corridor running east: walls all around, (1..=3, 1) empty.
fn corridor() -> FpView {
    let mut v = FpView::new(64, 32);
    #[rustfmt::skip]
    let cells = vec![
        1, 1, 1, 1, 1,
        1, 0, 0, 0, 1,
        1, 1, 1, 1, 1,
    ];
    v.set_map(5, 3, cells);
    v
}

#[test]
fn grid_directions() {
    assert_eq!((fp::dir_dx(0), fp::dir_dy(0)), (1, 0)); // east
    assert_eq!((fp::dir_dx(1), fp::dir_dy(1)), (0, -1)); // north = up on the map
    assert_eq!((fp::dir_dx(2), fp::dir_dy(2)), (-1, 0));
    assert_eq!((fp::dir_dx(3), fp::dir_dy(3)), (0, 1));
    assert_eq!((fp::dir_dx(-1), fp::dir_dy(5)), (0, -1), "directions wrap");
    assert_eq!(fp::angle_to_dir(0.0), 0);
    assert_eq!(fp::angle_to_dir(89.0), 1);
    assert_eq!(fp::angle_to_dir(-90.0), 3);
    assert_eq!(fp::angle_to_dir(540.0), 2);
}

#[test]
fn movement_respects_walls_and_edges() {
    let mut v = corridor();
    assert!(v.can_move(1, 1, 0), "east is open");
    assert!(!v.can_move(1, 1, 2), "west is a wall");
    assert!(!v.can_move(1, 1, 1) && !v.can_move(1, 1, 3));
    assert!(!v.can_move(3, 1, 0), "the end of the corridor");
    // a door between (2,1) and (3,1): set from either side, blocks both ways
    v.set_edge(3, 1, 2, 5);
    assert_eq!(v.edge(2, 1, 0), 5, "the same edge seen from the other cell");
    assert!(!v.can_move(2, 1, 0) && !v.can_move(3, 1, 2));
    // a non-solid edge type (open door / curtain) lets you pass
    v.walls.insert(5, fp::WallType { tex: None, color: 0xffffff, solid: false });
    assert!(v.can_move(2, 1, 0));
    v.set_edge(2, 1, 0, 0);
    assert_eq!(v.edge(3, 1, 2), 0, "type 0 removes it");
    // outside the map is solid; non-solid wall cells can be walked into
    let mut open = FpView::new(8, 8);
    open.set_map(2, 1, vec![0, 7]);
    assert!(!open.can_move(0, 0, 2));
    assert!(!open.can_move(0, 0, 0));
    open.walls.insert(7, fp::WallType { tex: None, color: 0, solid: false });
    assert!(open.can_move(0, 0, 0));
    assert_eq!(fp::edge_key(4, 4, 1), (4, 3, 3));
}

#[test]
fn geometry_has_only_visible_faces() {
    let mut v = FpView::new(16, 16);
    v.set_map(3, 3, vec![1, 1, 1, 1, 0, 1, 1, 1, 1]);
    let g = v.geometry(&[]);
    // 4 walls around the single empty cell + its floor + its ceiling
    assert_eq!(g.len(), 6);
    v.ceiling = fp::Fill::None;
    v.dirty();
    assert_eq!(v.geometry(&[]).len(), 5, "no ceiling: the background shows");
    v.set_edge(1, 1, 0, 2);
    assert_eq!(v.geometry(&[]).len(), 6, "an edge adds one quad");
    // the east wall face of cell (1,1) stands on x = 2, walls are 1 tall
    let east: Vec<_> = v.geometry(&[]).iter().filter(|q| q.p.iter().all(|p| (p[0] - 2.0).abs() < 1e-6)).cloned().collect();
    assert_eq!(east.len(), 2, "the wall face and the edge on the same side");
    let ys: Vec<f32> = east[0].p.iter().map(|p| p[1]).collect();
    assert_eq!(ys, vec![1.0, 1.0, 0.0, 0.0], "top-left, top-right, bottom-right, bottom-left");
}

#[test]
fn frames_cull_what_is_behind_or_too_far() {
    // a 30×30 hall with a pillar every 3 cells
    let mut v = FpView::new(64, 36);
    let cells: Vec<i32> = (0..900).map(|i| if (i % 30) % 3 == 0 && (i / 30) % 3 == 0 { 1 } else { 0 }).collect();
    v.set_map(30, 30, cells);
    v.fog = (0, 2.0, 6.0);
    v.cam = fp::Cam { x: 15.0, y: 15.0, angle: 0.0, height: 0.5, pitch: 0.0 };
    let all = v.geometry(&[]).len();
    let f = v.frame(&[]);
    assert!(f.faces.len() * 4 < all, "{} of {all}", f.faces.len());
    let (ex, ez) = (15.5f32, 15.5f32);
    for q in &f.faces {
        let (cx, cz) = ((q.p[0][0] + q.p[2][0]) / 2.0, (q.p[0][2] + q.p[2][2]) / 2.0);
        let d = ((cx - ex).powi(2) + (cz - ez).powi(2)).sqrt();
        assert!(d <= v.draw_distance() + 0.75, "too far: {d}");
        assert!(cx > ex - 1.5, "behind the camera: {cx}");
    }
    // grouped by texture for batching
    assert!(f.faces.windows(2).all(|w| w[0].tex.map(|t| t as i64).unwrap_or(-1) <= w[1].tex.map(|t| t as i64).unwrap_or(-1)));
}

#[test]
fn projection_and_rays_agree() {
    let mut v = corridor();
    v.cam = fp::Cam { x: 1.0, y: 1.0, angle: 0.0, height: 0.5, pitch: 0.0 };
    // a point straight ahead at eye height lands in the centre
    let (x, y, d) = v.project_world([4.5, 0.5, 1.5]).unwrap();
    assert!((x - 32.0).abs() < 1e-3 && (y - 16.0).abs() < 1e-3 && (d - 3.0).abs() < 1e-4, "{x} {y} {d}");
    // south (+z) is on the right when looking east; the floor is below the centre
    assert!(v.project_world([4.5, 0.5, 2.0]).unwrap().0 > 32.0);
    assert!(v.project_world([4.5, 0.0, 1.5]).unwrap().1 > 16.0);
    assert!(v.project_world([0.0, 0.5, 1.5]).is_none(), "behind the camera");
    // facing north (90°), east is on the right
    v.cam.angle = 90.0;
    assert!(v.project_world([2.5, 0.5, 0.0]).unwrap().0 > 32.0);
    // ray through a projected point goes back to it
    v.cam.angle = 30.0;
    let p = [3.0, 0.8, 0.9];
    let (px, py, _) = v.project_world(p).unwrap();
    let (o, dir) = v.ray(px, py);
    let t = ((p[0] - o[0]).powi(2) + (p[1] - o[1]).powi(2) + (p[2] - o[2]).powi(2)).sqrt();
    for k in 0..3 {
        assert!((o[k] + dir[k] * t - p[k]).abs() < 1e-3, "ray misses the point on axis {k}");
    }
    // the GPU matrix maps the same point to the same pixel (flip_y = false: y up in NDC)
    let m = v.matrix(false);
    let clip: Vec<f32> = (0..4).map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r]).collect();
    let (nx, ny) = (clip[0] / clip[3], clip[1] / clip[3]);
    assert!(((nx + 1.0) * 32.0 - px).abs() < 1e-2 && ((1.0 - ny) * 16.0 - py).abs() < 1e-2);
    let mf = v.matrix(true);
    assert!((mf[5] + m[5]).abs() < 1e-6, "flip_y negates the y row");
}

#[test]
fn picking_walls_floors_and_edges() {
    let mut v = corridor();
    v.cam = fp::Cam { x: 1.0, y: 1.0, angle: 0.0, height: 0.5, pitch: 0.0 };
    let h = v.pick(&[], 32.0, 16.0).expect("the wall at the end");
    assert_eq!((h.kind, h.cell_x, h.cell_y, h.dir), (PickKind::Wall, 4, 1, 2), "west face of (4,1)");
    assert!((h.dist - 2.5).abs() < 1e-3, "{}", h.dist);
    let f = v.pick(&[], 32.0, 31.0).expect("floor under the view");
    assert_eq!(f.kind, PickKind::Floor);
    let c = v.pick(&[], 32.0, 0.5).expect("ceiling");
    assert_eq!(c.kind, PickKind::Ceiling);
    v.set_edge(2, 1, 0, 3);
    let e = v.pick(&[], 32.0, 16.0).unwrap();
    assert_eq!((e.kind, e.cell_x, e.dir, e.value), (PickKind::Edge, 2, 0, 3));
    // looking at the side wall
    v.cam.angle = 90.0;
    let s = v.pick(&[], 32.0, 16.0).unwrap();
    assert_eq!((s.kind, s.cell_x, s.cell_y, s.dir), (PickKind::Wall, 1, 0, 3));
}

#[test]
fn triangulation_of_primitives() {
    let p = |x: f32, y: f32| MVert { x, y, u: 0.0, v: 0.0, color: [255; 4] };
    let quad = [p(0.0, 0.0), p(10.0, 0.0), p(0.0, 10.0), p(10.0, 10.0)];
    assert_eq!(fp::triangulate(4, &quad[..3]).len(), 3);
    assert_eq!(fp::triangulate(4, &quad).len(), 3, "an incomplete triangle is dropped");
    assert_eq!(fp::triangulate(5, &quad).len(), 6, "strip of 4 = 2 triangles");
    assert_eq!(fp::triangulate(6, &quad).len(), 6, "fan of 4 = 2 triangles");
    assert_eq!(fp::triangulate(2, &quad).len(), 12, "2 lines = 2 quads");
    assert_eq!(fp::triangulate(3, &quad).len(), 18, "strip of 3 lines");
    assert_eq!(fp::triangulate(1, &quad).len(), 24, "4 points = 4 little squares");
    let line = fp::triangulate(2, &[p(0.0, 5.0), p(10.0, 5.0)]);
    let ys: Vec<f32> = line.iter().map(|v| v.y).collect();
    assert!(ys.iter().all(|y| (*y - 4.5).abs() < 1e-6 || (*y - 5.5).abs() < 1e-6), "1 px wide: {ys:?}");
}

#[test]
fn sprite_pos_mapping() {
    let q = [[0.0, 0.0], [100.0, 20.0], [100.0, 80.0], [0.0, 100.0]];
    let map = fp::square_to_quad(q);
    for (uv, want) in [((0.0, 0.0), q[0]), ((1.0, 0.0), q[1]), ((1.0, 1.0), q[2]), ((0.0, 1.0), q[3])] {
        let (x, y) = map(uv.0, uv.1);
        assert!((x - want[0]).abs() < 1e-3 && (y - want[1]).abs() < 1e-3, "{uv:?} -> {x},{y}");
    }
    // projective: the centre of the texture goes to where the diagonals cross (a trapezoid
    // whose right side is shorter = further away, so the centre is right of x = 50)
    let (cx, cy) = map(0.5, 0.5);
    assert!(cx > 55.0 && (cy - 50.0).abs() < 1e-3, "{cx} {cy}");
    let affine = fp::sprite_pos_tris(q, [0.0, 0.0, 1.0, 1.0], [255; 4], false);
    assert_eq!(affine.len(), 6);
    assert_eq!((affine[2].x, affine[2].y, affine[2].u, affine[2].v), (100.0, 80.0, 1.0, 1.0));
    let persp = fp::sprite_pos_tris(q, [0.25, 0.0, 0.5, 1.0], [255; 4], true);
    assert_eq!(persp.len(), 8 * 8 * 6);
    assert!(persp.iter().all(|v| (0.25..=0.5).contains(&v.u)), "UVs stay inside the frame");
}

#[test]
fn transition_widths() {
    assert_eq!(fp::transition_width(320.0, 0.0), 320);
    assert_eq!(fp::transition_width(320.0, 1.0), fp::TRANSITION_MIN_WIDTH as u32);
    let mid = fp::transition_width(320.0, 0.5);
    assert!(mid > 6 && mid < 320);
    assert!(fp::transition_width(320.0, 0.75) < mid);
}

// ------------------------------------------------------------------ through ModScript

fn project(tag: &str, obj: &str) -> (PathBuf, Engine) {
    let dir = std::env::temp_dir().join(format!("modengine_fpview_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "game.toml", "[game]\nname = \"fp\"\n[display]\nwidth = 64\nheight = 32\n[palette]\n\"a\" = \"#ff0000\"\n\"b\" = \"#00ff00\"\n");
    // a 4×4 sprite: opaque left half, transparent right half
    write(&dir, "sprites/half.toml", "pixels = ['''\naa..\naa..\naa..\naa..\n''']\n");
    write(&dir, "sprites/stone.toml", "pixels = ['''\nbb\nbb\n''']\n");
    write(&dir, "rooms/r.toml", "[[layer]]\ntype = \"instances\"\n[[layer.instances]]\nobject = \"T\"\nx = 0\ny = 0\n");
    write(&dir, "objects/t.ms", obj);
    let e = Engine::load(&dir).unwrap_or_else(|m| panic!("{tag}: {m}"));
    (dir, e)
}

fn run(e: &mut Engine, steps: u32) {
    e.start();
    for _ in 0..steps {
        e.tick();
        e.render();
        e.audio.clear();
    }
}

fn g(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or_else(|| panic!("global.{name} not set; errors: {:?}", e.errors))
}

fn field(v: &Value, k: &str) -> Value {
    match v {
        Value::Struct(s) => s.borrow().fields.get(&intern(k)).cloned().unwrap_or(Value::Null),
        other => panic!("not a struct: {other:?}"),
    }
}

const MAP: &str = "global.MAP = [[1,1,1,1,1,1],[1,0,0,0,0,1],[1,1,1,1,1,1]]\n";

#[test]
fn script_picks_billboards_only_where_opaque_and_in_front() {
    let src = format!(
        "{MAP}object T {{
    on create {{
        global.v = fpview_create(64, 32)
        fpview_set_map(global.v, global.MAP)
        fpview_set_camera(global.v, 1, 1, 0)
        global.i = fpview_add_billboard(global.v, 3, 1, half, 0, 1)
        fpview_draw(global.v, 0, 0, 64, 32)
        // the opaque (left) half of the sprite is left of the centre
        global.hit = fpview_pick(global.v, 30, 16)
        // the transparent half: the ray goes on to the wall behind
        global.miss = fpview_pick(global.v, 34, 16)
        global.can = fpview_can_move(global.v, 1, 1, 0)
        global.cant = fpview_can_move(global.v, 1, 1, 1)
        global.proj = fpview_project(global.v, 3, 1)
        // drawn at twice the size: pick coordinates follow the last fpview_draw
        fpview_draw(global.v, 100, 0, 128, 64)
        global.hit2 = fpview_pick(global.v, 100 + 60, 32)
        global.outside = fpview_pick(global.v, 10, 10)
        // a wall in between hides it
        fpview_set_cell(global.v, 2, 1, 1)
        global.hidden = fpview_pick(global.v, 160, 32)
    }}
}}
"
    );
    let (_d, mut e) = project("pick", &src);
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let hit = g(&e, "hit");
    assert_eq!(field(&hit, "kind").to_string(), "billboard");
    assert_eq!(field(&hit, "index").as_num(), g(&e, "i").as_num());
    assert_eq!(field(&g(&e, "miss"), "kind").to_string(), "wall");
    assert_eq!(field(&g(&e, "miss"), "cell_x").as_num(), Some(5.0));
    assert!(g(&e, "can").truthy() && !g(&e, "cant").truthy());
    let proj = g(&e, "proj");
    assert!((field(&proj, "x").as_num().unwrap() - 32.0).abs() < 1e-3);
    assert!((field(&proj, "depth").as_num().unwrap() - 2.0).abs() < 1e-3);
    assert_eq!(field(&g(&e, "hit2"), "kind").to_string(), "billboard");
    assert!(g(&e, "outside").is_null());
    let hidden = g(&e, "hidden");
    assert_eq!((field(&hidden, "kind").to_string(), field(&hidden, "cell_x").as_num()), ("wall".to_string(), Some(2.0)));
    // fpview_draw records one FpView command with the visible faces
    let views: Vec<_> = e.cmds.iter().filter_map(|c| if let DrawCmd::FpView { frame, w, .. } = c { Some((frame.faces.len(), frame.sprites.len(), *w)) } else { None }).collect();
    assert!(views.is_empty(), "drawn in create: the frame's list starts at render; got {views:?}");
}

#[test]
fn script_draw_records_faces_and_billboards() {
    let src = format!(
        "{MAP}object T {{
    on create {{
        global.v = fpview_create(32, 16)
        fpview_set_map(global.v, global.MAP)
        fpview_set_wall(global.v, 1, stone)
        fpview_set_floor(global.v, #404040)
        fpview_set_ceiling(global.v, null)
        fpview_set_fog(global.v, #102030, 1, 4)
    }}
    on step {{
        fpview_set_camera(global.v, 1, 1, 0)
        fpview_clear_billboards(global.v)
        fpview_add_billboard(global.v, 3, 1, half)
        fpview_add_billboard(global.v, 2, 1, half, 0, 0.5)
    }}
    on draw {{ fpview_draw(global.v, 0, 0) }}
}}
"
    );
    let (_d, mut e) = project("draw", &src);
    run(&mut e, 2);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let f = e.cmds.iter().find_map(|c| if let DrawCmd::FpView { frame, w, h, .. } = c { Some((frame.clone(), *w, *h)) } else { None }).expect("an FpView command");
    assert_eq!((f.1, f.2), (32.0, 16.0), "default size = the view's");
    let fr = f.0;
    assert_eq!((fr.w, fr.h), (32, 16));
    assert_eq!(fr.sprites.len(), 2);
    // far to near: the first billboard drawn is the one at x = 3
    assert!(fr.sprites[0].p[0][0] > fr.sprites[1].p[0][0]);
    assert!(fr.faces.iter().any(|q| q.tex.is_some()), "textured walls");
    assert!(fr.faces.iter().any(|q| q.tex.is_none() && q.p.iter().all(|p| p[1] == 0.0)), "colour floor");
    assert!(!fr.faces.iter().any(|q| q.p.iter().all(|p| p[1] == 1.0)), "no ceiling");
    assert_eq!(fr.fog_range, [1.0, 4.0]);
}

#[test]
fn script_primitives_and_errors() {
    let src = "object T {
    on draw {
        draw_primitive_begin(pr_trianglestrip)
        draw_vertex(0, 0)
        draw_vertex_color(10, 0, #ff0000, 0.5)
        draw_vertex(0, 10)
        draw_vertex(10, 10)
        draw_primitive_end()
        draw_primitive_begin_texture(pr_trianglelist, half, 0)
        draw_vertex_texture(0, 0, 0, 0)
        draw_vertex_texture_colour(4, 0, 1, 0, #00ff00, 1)
        draw_vertex_texture(4, 4, 1, 1)
        draw_primitive_end()
        draw_sprite_pos(stone, 0, 0, 0, 10, 2, 10, 8, 0, 10, 0.5, true)
    }
}
";
    let (_d, mut e) = project("prim", src);
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let meshes: Vec<_> = e.cmds.iter().filter_map(|c| if let DrawCmd::Mesh { tex, verts } = c { Some((*tex, verts.clone())) } else { None }).collect();
    assert_eq!(meshes.len(), 3);
    assert_eq!((meshes[0].0, meshes[0].1.len()), (None, 6));
    assert!(meshes[0].1.iter().any(|v| v.color == [255, 0, 0, 128]), "vertex colour + alpha");
    assert!(meshes[0].1.iter().any(|v| v.color == [255, 255, 255, 255]), "draw_vertex uses the draw colour");
    assert!(meshes[1].0.is_some() && meshes[1].1.len() == 3);
    assert_eq!(meshes[2].1.len(), 8 * 8 * 6);
    assert!(meshes[2].1.iter().all(|v| v.color[3] == 128));

    let bad = "object T {\n    on create {\n        draw_vertex(0, 0)\n    }\n}\n";
    let (_d, mut e) = project("prim_err", bad);
    run(&mut e, 1);
    assert!(e.errors.iter().any(|m| m.contains("draw_primitive_begin")), "{:?}", e.errors);
}

#[test]
fn script_postfx_goes_before_the_gui() {
    let src = "object T {
    on create { postfx_pixelate(16, 4, 0.5, [#000000, #ffffff]) }
    on step {
        if steps() == 3 postfx_pixelate(8, 0, 0, true, true)
        if steps() == 4 { postfx_off(); postfx_transition(1) }
    }
    on draw_gui { draw_rect(0, 0, 4, 4) }
}
";
    let (_d, mut e) = project("postfx", src);
    let idx = |e: &Engine| {
        let gui = e.cmds.iter().position(|c| matches!(c, DrawCmd::Gui)).unwrap();
        let fx = e.cmds.iter().position(|c| matches!(c, DrawCmd::PostFx { .. }));
        (gui, fx, e.cmds.iter().filter(|c| matches!(c, DrawCmd::PostFx { .. })).count())
    };
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    let (gui, fx, n) = idx(&e);
    assert_eq!(n, 1);
    assert!(fx.unwrap() < gui, "world only by default");
    match &e.cmds[fx.unwrap()] {
        DrawCmd::PostFx { pixels_wide, levels, dither, palette } => {
            assert_eq!((*pixels_wide, *levels, *dither, palette.as_slice()), (16, 4, 0.5, &[0x000000, 0xffffff][..]));
        }
        _ => unreachable!(),
    }
    for _ in 0..2 {
        e.tick();
        e.render();
    }
    let (gui, fx, _) = idx(&e);
    assert!(fx.unwrap() > gui, "include_gui puts it after the GUI");
    if let DrawCmd::PostFx { palette, .. } = &e.cmds[fx.unwrap()] {
        assert_eq!(palette.as_slice(), &[0xff0000, 0x00ff00][..], "true = the game palette");
    }
    e.tick();
    e.render();
    let (_, fx, _) = idx(&e);
    match &e.cmds[fx.expect("the transition alone still pixelates")] {
        DrawCmd::PostFx { pixels_wide, levels, .. } => assert_eq!((*pixels_wide, *levels), (fp::TRANSITION_MIN_WIDTH as u32, 0)),
        _ => unreachable!(),
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn script_minimap_and_grids() {
    let src = format!(
        "{MAP}object T {{
    on create {{
        global.v = fpview_create(32, 16)
        var gr = ds_grid_create(6, 3)
        ds_grid_set(gr, 0, 0, 1)
        fpview_set_map(global.v, gr)
        global.cell = fpview_get_cell(global.v, 0, 0)
        global.w = fpview_get_cell(global.v, 5, 2)
        fpview_set_map(global.v, global.MAP)
        fpview_set_edge(global.v, 2, 1, 0, 1)
        fpview_set_camera(global.v, 1, 1, 0)
    }}
    on draw_gui {{
        draw_minimap(global.MAP, 0, 0, 4, {{ player_x: 1, player_y: 1, dir: 0, width: 12, height: 8 }})
        fpview_draw_minimap(global.v, 30, 0, 2)
    }}
}}
"
    );
    let (_d, mut e) = project("minimap", &src);
    run(&mut e, 1);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
    assert_eq!(g(&e, "cell").as_num(), Some(1.0), "a ds_grid works as a map");
    assert_eq!(g(&e, "w").as_num(), Some(0.0));
    let gui = e.cmds.iter().position(|c| matches!(c, DrawCmd::Gui)).unwrap();
    let after = &e.cmds[gui..];
    let scissors: Vec<_> = after.iter().filter_map(|c| if let DrawCmd::Scissor(r) = c { Some(*r) } else { None }).collect();
    assert_eq!(scissors, vec![Some([0.0, 0.0, 12.0, 8.0]), None], "clipped to the window, then restored");
    let rects = after.iter().filter(|c| matches!(c, DrawCmd::Rect { .. })).count();
    // the clipped one draws only the cells in its window (≤ 4×3 + 1), the other all 18
    assert!(rects > 18 && rects <= 18 + 4 * 4, "{rects}");
    assert_eq!(after.iter().filter(|c| matches!(c, DrawCmd::Tri { .. })).count(), 2, "two player arrows");
    assert_eq!(after.iter().filter(|c| matches!(c, DrawCmd::Line { .. })).count(), 1, "the edge");
}

#[test]
fn bad_arguments_are_errors() {
    for (code, needle) in [
        ("fpview_draw(99, 0, 0)", "does not exist"),
        ("fpview_set_map(fpview_create(8, 8), 3)", "not a ds_grid"),
        ("fpview_set_wall(fpview_create(8, 8), 0, stone)", "wall types"),
        ("postfx_pixelate(0)", "pixels_wide"),
        ("draw_primitive_begin(9)", "unknown kind"),
        ("fpview_create(0, 10)", "out of range"),
    ] {
        let src = format!("object T {{\n    on create {{\n        {code}\n    }}\n}}\n");
        let (_d, mut e) = project("bad", &src);
        run(&mut e, 1);
        assert!(e.errors.iter().any(|m| m.contains(needle)), "{code}: {:?}", e.errors);
    }
}
