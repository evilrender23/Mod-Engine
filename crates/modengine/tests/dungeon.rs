//! ds_grid (2D grids, serialization, `grid[# x, y]`), Delaunay / MST, and the procedural
//! dungeon generator: determinism, connectivity, room overlap, loops, event rules, speed.

use modengine::dungeon::{self as dg, Config, ParseCtx, CORRIDOR, DOOR, FLOOR, WALL};
use modengine::Engine;
use modscript::{intern, Value};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn project(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modengine-dungeon-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["rooms", "sprites", "objects"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("game.toml"), "[game]\nname = \"dg\"\nrooms = [\"r\"]\n\n[display]\nwidth = 320\nheight = 240\n\n[palette]\n\"w\" = \"#ffffff\"\n").unwrap();
    std::fs::write(dir.join("sprites/dot.toml"), "pixels = ['''\nw\n''']\n").unwrap();
    std::fs::write(dir.join("rooms/r.toml"), "width = 320\nheight = 240\n").unwrap();
    for (name, data) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }
    dir
}

fn start(dir: &Path) -> Engine {
    let mut e = Engine::load(dir).unwrap_or_else(|err| panic!("{err}"));
    e.save_dir = dir.join("saves");
    e.start();
    e
}

fn run(e: &mut Engine, code: &str) {
    e.run_code(code).unwrap_or_else(|err| panic!("{err}\n{code}"));
}

fn g(e: &Engine, name: &str) -> Value {
    e.globals.get(&intern(name)).cloned().unwrap_or(Value::Null)
}

fn num(e: &Engine, name: &str) -> f64 {
    g(e, name).as_num().unwrap_or_else(|| panic!("global.{name} = {}", g(e, name)))
}

// ------------------------------------------------------------------ ds_grid

#[test]
fn ds_grid_basic_operations() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var gr = ds_grid_create(5, 4)
        global.w = ds_grid_width(gr)
        global.h = ds_grid_height(gr)
        ds_grid_clear(gr, 1)
        ds_grid_set(gr, 2, 1, 7)
        ds_grid_add(gr, 2, 1, 3)
        ds_grid_multiply(gr, 0, 0, 5)
        global.v = ds_grid_get(gr, 2, 1)
        global.m = ds_grid_get(gr, 0, 0)
        global.outside = ds_grid_get(gr, 9, 9)
        ds_grid_set_region(gr, 3, 2, 4, 3, 2)
        global.sum = ds_grid_get_sum(gr, 0, 0, 4, 3)
        global.max = ds_grid_get_max(gr, 0, 0, 4, 3)
        global.min = ds_grid_get_min(gr, 3, 2, 4, 3)
        global.mean = ds_grid_get_mean(gr, 3, 2, 4, 3)
        global.ex = ds_grid_value_exists(gr, 0, 0, 4, 3, 10)
        global.vx = ds_grid_value_x(gr, 0, 0, 4, 3, 10)
        global.vy = ds_grid_value_y(gr, 0, 0, 4, 3, 10)
        global.nx = ds_grid_value_x(gr, 0, 0, 4, 3, 99)
        global.cnt = ds_grid_count(gr, 2)
        ds_grid_add_region(gr, 0, 3, 1, 3, 100)
        global.ar = ds_grid_get(gr, 1, 3)
        ds_grid_resize(gr, 3, 2)
        global.rw = ds_grid_width(gr)
        global.kept = ds_grid_get(gr, 2, 1)
        var c = ds_grid_create(1, 1)
        ds_grid_copy(c, gr)
        global.cw = ds_grid_width(c)
        global.cv = ds_grid_get(c, 2, 1)
        ds_grid_destroy(c)
        global.alive = ds_grid_exists(c)
        global.gr = gr
        "#,
    );
    assert_eq!((num(&e, "w"), num(&e, "h")), (5.0, 4.0));
    assert_eq!(num(&e, "v"), 10.0);
    assert_eq!(num(&e, "m"), 5.0);
    assert!(g(&e, "outside").is_null());
    // 20 cells: 15 of 1 except (0,0)=5, (2,1)=10 → 1*13 + 5 + 10 = 28 before region; region 4 cells of 2
    // 14 cells of 1, (0,0) = 5, (2,1) = 10, a 2×2 region of 2
    assert_eq!(num(&e, "sum"), 14.0 + 5.0 + 10.0 + 8.0);
    assert_eq!(num(&e, "max"), 10.0);
    assert_eq!((num(&e, "min"), num(&e, "mean")), (2.0, 2.0));
    assert!(g(&e, "ex").truthy());
    assert_eq!((num(&e, "vx"), num(&e, "vy"), num(&e, "nx")), (2.0, 1.0, -1.0));
    assert_eq!(num(&e, "cnt"), 4.0);
    assert_eq!(num(&e, "ar"), 101.0);
    assert_eq!((num(&e, "rw"), num(&e, "kept")), (3.0, 10.0));
    assert_eq!((num(&e, "cw"), num(&e, "cv")), (3.0, 10.0));
    assert!(!g(&e, "alive").truthy());
    // the Rust accessor other natives use
    let id = num(&e, "gr");
    let grid = e.ds_grids.get(id).expect("grid");
    assert_eq!((grid.w, grid.h), (3, 2));
    assert_eq!(grid.to_i32(-1)[1 * 3 + 2], 10);
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn ds_grid_accessor_syntax() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var gr = ds_grid_create(4, 4)
        gr[# 1, 2] = 5
        gr[# 1, 2] += 3
        gr[# 1, 2] *= 2
        gr[# 1, 2] -= 1
        gr[# 0, 0] = "a"
        gr[# 0, 0] += "b"
        gr[# 3, 3]++
        gr[# 3, 3]++
        gr[# 3, 3]--
        gr[# 2, 2] = 9
        gr[# 2, 2] /= 3
        global.a = gr[# 1, 2]
        global.s = gr[# 0, 0]
        global.c = gr[# 3, 3]
        global.d = gr[# 2, 2]
        global.expr = gr[# 1 + 0, 1 * 2] + 1
        global.arr = [#FF0000, 2]
        "#,
    );
    assert_eq!(num(&e, "a"), 15.0);
    assert_eq!(g(&e, "s").to_string(), "ab");
    assert_eq!(num(&e, "c"), 1.0);
    assert_eq!(num(&e, "d"), 3.0);
    assert_eq!(num(&e, "expr"), 16.0);
    // colour literals in arrays still work
    let Value::Array(a) = g(&e, "arr") else { panic!() };
    assert_eq!(a.borrow()[0].as_num(), Some(0xFF0000 as f64));
}

#[test]
fn ds_grid_serialization_arrays_and_shuffle() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var gr = ds_grid_create(6, 3)
        ds_grid_set(gr, 0, 0, "texto")
        ds_grid_set(gr, 5, 2, 3.5)
        ds_grid_set(gr, 2, 1, [1, 2])
        global.txt = ds_grid_write(gr)
        var b = ds_grid_create(1, 1)
        ds_grid_read(b, global.txt)
        global.bw = ds_grid_width(b)
        global.b00 = ds_grid_get(b, 0, 0)
        global.b52 = ds_grid_get(b, 5, 2)
        global.b21 = ds_grid_get(b, 2, 1)[1]
        global.same = ds_grid_write(b) == global.txt
        global.rows = ds_grid_to_array(b)
        var c = ds_grid_from_array([[1, 2, 3], [4, 5, 6]])
        global.c12 = ds_grid_get(c, 2, 1)
        global.cw = ds_grid_width(c)
        // shuffle: same values, reproducible with a stream
        var r1 = rng_create(5)
        var r2 = rng_create(5)
        var s1 = ds_grid_from_array([[1, 2, 3, 4], [5, 6, 7, 8]])
        var s2 = ds_grid_from_array([[1, 2, 3, 4], [5, 6, 7, 8]])
        ds_grid_shuffle(s1, r1)
        ds_grid_shuffle(s2, r2)
        global.shuf_same = ds_grid_write(s1) == ds_grid_write(s2)
        global.shuf_sum = ds_grid_get_sum(s1, 0, 0, 3, 1)
        ds_grid_sort(c, 0, false)
        global.sorted = ds_grid_get(c, 0, 0)
        "#,
    );
    assert_eq!(num(&e, "bw"), 6.0);
    assert_eq!(g(&e, "b00").to_string(), "texto");
    assert_eq!(num(&e, "b52"), 3.5);
    assert_eq!(num(&e, "b21"), 2.0);
    assert!(g(&e, "same").truthy());
    // runs of equal values keep the text short
    assert!(g(&e, "txt").to_string().len() < 120, "{}", g(&e, "txt"));
    let Value::Array(rows) = g(&e, "rows") else { panic!() };
    assert_eq!(rows.borrow().len(), 3);
    assert_eq!((num(&e, "c12"), num(&e, "cw")), (6.0, 3.0));
    assert!(g(&e, "shuf_same").truthy());
    assert_eq!(num(&e, "shuf_sum"), 36.0);
    assert_eq!(num(&e, "sorted"), 4.0);
    // a bad text is an error, not a crash
    assert!(e.run_code("var b = ds_grid_create(1, 1)\nds_grid_read(b, \"hola\")").is_err());
    assert!(e.run_code("ds_grid_get(99, 0, 0)").is_err());
}

#[test]
fn mp_grid_from_a_ds_grid() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var gr = ds_grid_from_array([[1, 1, 1, 1], [1, 0, 3, 1], [1, 2, 1, 1]])
        var mp = mp_grid_create(0, 0, 4, 3, 16, 16)
        global.n = mp_grid_add_grid(mp, gr)
        global.free = mp_grid_get_cell(mp, 1, 1)
        global.wall = mp_grid_get_cell(mp, 0, 0)
        var mp2 = mp_grid_create(0, 0, 4, 3, 16, 16)
        global.n2 = mp_grid_add_grid(mp2, gr, [1, 2])
        "#,
    );
    assert_eq!(num(&e, "n"), 9.0);
    assert_eq!(num(&e, "free"), 0.0);
    assert_eq!(num(&e, "wall"), -1.0);
    assert_eq!(num(&e, "n2"), 10.0);
}

// ------------------------------------------------------------------ Delaunay / MST

fn lcg_points(n: usize, seed: u64) -> Vec<(f64, f64)> {
    let mut r = dg::Rng::new(seed);
    (0..n).map(|_| ((r.f() * 100.0).round(), (r.f() * 100.0).round())).collect()
}

fn check_delaunay(pts: &[(f64, f64)]) -> usize {
    let tris = dg::delaunay(pts);
    for t in &tris {
        let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
        let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
        assert!(d.abs() > 1e-9, "degenerate triangle {t:?}");
        let (a2, b2, c2) = (a.0 * a.0 + a.1 * a.1, b.0 * b.0 + b.1 * b.1, c.0 * c.0 + c.1 * c.1);
        let ux = (a2 * (b.1 - c.1) + b2 * (c.1 - a.1) + c2 * (a.1 - b.1)) / d;
        let uy = (a2 * (c.0 - b.0) + b2 * (a.0 - c.0) + c2 * (b.0 - a.0)) / d;
        let r2 = (a.0 - ux).powi(2) + (a.1 - uy).powi(2);
        for (i, p) in pts.iter().enumerate() {
            if t.contains(&i) {
                continue;
            }
            let d2 = (p.0 - ux).powi(2) + (p.1 - uy).powi(2);
            assert!(d2 >= r2 * (1.0 - 1e-7), "point {i} {p:?} inside the circumcircle of {t:?}");
        }
    }
    tris.len()
}

#[test]
fn delaunay_has_empty_circumcircles() {
    // a square with its centre: 4 triangles
    let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (5.0, 5.0)];
    assert_eq!(check_delaunay(&sq), 4);
    // co-circular grid points (degenerate on purpose): still valid, 2 triangles per cell
    let grid: Vec<(f64, f64)> = (0..4).flat_map(|y| (0..4).map(move |x| (x as f64 * 3.0, y as f64 * 3.0))).collect();
    assert_eq!(check_delaunay(&grid), 18);
    for seed in 0..40 {
        let pts = lcg_points(5 + seed as usize % 30, seed);
        let n = check_delaunay(&pts);
        assert!(n > 0);
    }
    // Euler: a triangulation of n points with h on the hull has 2n - 2 - h triangles
    let tri = [(0.0, 0.0), (8.0, 0.0), (4.0, 7.0), (4.0, 2.0), (3.0, 3.0)];
    assert_eq!(check_delaunay(&tri), 2 * 5 - 2 - 3);
    // collinear and duplicates: no triangles, but the edges still connect everything
    let line = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (5.0, 0.0), (2.0, 0.0)];
    assert!(dg::delaunay(&line).is_empty());
    let e = dg::delaunay_edges(&line);
    assert_eq!(dg::mst(5, &e, |a, b| dg::dist(line[a], line[b])).len(), 4);
}

fn brute_mst_weight(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    let edges: Vec<(usize, usize)> = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b))).collect();
    let m = edges.len();
    let mut best = f64::INFINITY;
    // every subset of n-1 edges that spans the points
    let mut idx: Vec<usize> = (0..n - 1).collect();
    loop {
        let mut uf = dg::UnionFind::new(n);
        if idx.iter().all(|&k| uf.union(edges[k].0, edges[k].1)) {
            best = best.min(idx.iter().map(|&k| dg::dist(pts[edges[k].0], pts[edges[k].1])).sum());
        }
        // next combination
        let mut i = n - 1;
        loop {
            if i == 0 {
                return best;
            }
            i -= 1;
            if idx[i] != i + m - (n - 1) {
                break;
            }
            if i == 0 && idx[0] == m - (n - 1) {
                return best;
            }
        }
        idx[i] += 1;
        for j in i + 1..n - 1 {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

#[test]
fn mst_matches_brute_force() {
    for seed in 0..25 {
        let n = 3 + seed as usize % 4;
        let pts = lcg_points(n, 100 + seed);
        let tree = dg::mst(n, &dg::delaunay_edges(&pts), |a, b| dg::dist(pts[a], pts[b]));
        assert_eq!(tree.len(), n - 1);
        let w: f64 = tree.iter().map(|&(a, b)| dg::dist(pts[a], pts[b])).sum();
        let bw = brute_mst_weight(&pts);
        assert!((w - bw).abs() < 1e-6, "seed {seed}: mst {w} vs brute force {bw}");
    }
}

#[test]
fn delaunay_and_mst_from_scripts() {
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var pts = [[0, 0], [10, 0], [10, 10], [0, 10], { x: 5, y: 5 }]
        var d = delaunay(pts)
        global.tris = len(d.triangles)
        global.edges = len(d.edges)
        global.tree = len(mst(pts))
        global.tree2 = len(mst(pts, [[0, 1], [1, 2], [2, 3]]))
        "#,
    );
    assert_eq!(num(&e, "tris"), 4.0);
    assert_eq!(num(&e, "edges"), 8.0);
    assert_eq!(num(&e, "tree"), 4.0);
    // a forest when the edges do not connect everything
    assert_eq!(num(&e, "tree2"), 3.0);
}

// ------------------------------------------------------------------ generator

fn cfg(j: serde_json::Value) -> Config {
    let mut w = vec![];
    let c = Config::parse(&j, &ParseCtx { base: None, room_loader: None }, &mut w).unwrap_or_else(|m| panic!("{m}"));
    assert!(w.is_empty(), "{w:?}");
    c
}

fn demo() -> Config {
    cfg(json!({
        "width": 64, "height": 64, "rooms": [12, 15],
        "room_size": { "min": [4, 4], "max": [8, 7] },
        "extra_edges": 0.25,
        "rect_weight": 2,
        "templates": [
            { "name": "cruz", "rotate": true, "rows": ["  ...  ", "  .E.  ", ".......", ".E.C.E.", ".......", "  ...  "] },
            { "name": "salon", "rows": "#########\n#.......#\n#.E...E.#\nD...C...D\n#.......#\n#########" },
            { "name": "trono", "min": 1, "max": 1, "tags": ["boss"], "rows": [".......", "...B...", ".......", "...X..."] }
        ],
        "events": [
            { "type": "enemy", "count": [5, 8], "where": "room", "marker": "E", "min_distance_from_start": 6 },
            { "type": "chest", "count": [1, 3], "where": "dead_end", "marker": "C" },
            { "type": "boss", "where": "exit_room", "marker": "B" },
            { "type": [{ "type": "shop", "weight": 1 }, { "type": "well", "weight": 2 }], "count": 2, "where": "room", "on_main_path": false },
            { "type": "trap", "count": [3, 4], "where": "corridor", "min_spacing": 3 }
        ]
    }))
}

fn check_rooms_dont_overlap(d: &dg::Dungeon) {
    for (i, a) in d.rooms.iter().enumerate() {
        assert!(a.x >= 1 && a.y >= 1 && a.x + a.w <= d.w - 1 && a.y + a.h <= d.h - 1, "room {i} outside the map");
        for (j, b) in d.rooms.iter().enumerate().skip(i + 1) {
            let sep = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
            assert!(sep, "rooms {i} and {j} overlap");
        }
    }
}

#[test]
fn same_seed_same_dungeon_and_different_seeds_differ() {
    let c = demo();
    let a = dg::generate(&c, 42).unwrap();
    let b = dg::generate(&c, 42).unwrap();
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.cells, b.cells);
    assert_eq!(a.events.len(), b.events.len());
    let other = dg::generate(&c, 43).unwrap();
    assert_ne!(a.hash(), other.hash());
}

#[test]
fn every_floor_cell_is_reachable_and_rooms_do_not_overlap() {
    let c = demo();
    for seed in 0..30 {
        let d = dg::generate(&c, seed).unwrap();
        assert!(d.rooms.len() >= 12 && d.rooms.len() <= 15, "seed {seed}: {} rooms", d.rooms.len());
        check_rooms_dont_overlap(&d);
        // borders are walls
        for x in 0..d.w {
            assert_eq!(d.cells[x], WALL);
            assert_eq!(d.cells[(d.h - 1) * d.w + x], WALL);
        }
        for (i, &cell) in d.cells.iter().enumerate() {
            if cell != WALL {
                assert!(d.dist[i] >= 0, "seed {seed}: cell {:?} ({cell}) not reachable", (i % d.w, i / d.w));
            }
        }
        // the main path goes from start to exit through walkable, adjacent cells
        assert_eq!(d.main_path.first(), Some(&d.start));
        assert_eq!(d.main_path.last(), Some(&d.exit));
        for w in d.main_path.windows(2) {
            assert_eq!(w[0].0.abs_diff(w[1].0) + w[0].1.abs_diff(w[1].1), 1);
        }
        assert_eq!(d.main_path.len() as i32, d.distance(d.exit.0 as i64, d.exit.1 as i64) + 1);
        // the boss room template's X marker is the exit
        assert_eq!(d.rooms[d.exit_room].template, "trono", "seed {seed}");
        assert!(d.rooms[d.exit_room].on_main_path && d.rooms[d.start_room].on_main_path);
        assert!(d.cells.contains(&DOOR) && d.cells.contains(&CORRIDOR) && d.cells.contains(&FLOOR));
        assert!(d.warnings.iter().all(|w| !w.contains("no se pudo unir")), "{:?}", d.warnings);
    }
}

#[test]
fn graph_is_mst_plus_the_asked_share_of_loops() {
    for (extra, limit) in [(0.0, 2.0), (0.3, 2.0), (1.0, 2.0), (1.0, 0.0)] {
        let mut c = demo();
        c.extra_edges = extra;
        c.extra_max_length = limit;
        for seed in 0..10 {
            let d = dg::generate(&c, seed).unwrap();
            let n = d.rooms.len();
            assert_eq!(d.mst.len(), n - 1);
            let unused = d.delaunay.len() - d.mst.len();
            assert!(d.loop_candidates <= unused);
            if limit == 0.0 {
                assert_eq!(d.loop_candidates, unused);
            }
            assert_eq!(d.loops(), (extra * d.loop_candidates as f64).round() as usize);
            // loops are short: at most extra_max_length times the tree's average edge
            let pts: Vec<(f64, f64)> = d.rooms.iter().map(|r| (r.center.0 as f64, r.center.1 as f64)).collect();
            let avg = d.mst.iter().map(|&(a, b)| dg::dist(pts[a], pts[b])).sum::<f64>() / d.mst.len() as f64;
            assert!(limit == 0.0 || d.extra.iter().all(|&(a, b)| dg::dist(pts[a], pts[b]) <= avg * limit + 1e-9));
            // independent cycles of the room graph = edges - vertices + 1
            let edges = d.edges();
            let mut uniq = edges.clone();
            uniq.sort_unstable();
            uniq.dedup();
            assert_eq!(uniq.len() - n + 1, d.loops());
            // the MST is minimal over the Delaunay graph (same weight as over all pairs)
            let pts: Vec<(f64, f64)> = d.rooms.iter().map(|r| (r.center.0 as f64, r.center.1 as f64)).collect();
            let all: Vec<(usize, usize)> = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b))).collect();
            let wsum = |t: &[(usize, usize)]| t.iter().map(|&(a, b)| dg::dist(pts[a], pts[b])).sum::<f64>();
            assert!((wsum(&d.mst) - wsum(&dg::mst(n, &all, |a, b| dg::dist(pts[a], pts[b])))).abs() < 1e-6);
        }
    }
}

#[test]
fn events_follow_their_rules() {
    let c = demo();
    for seed in 0..30 {
        let d = dg::generate(&c, seed).unwrap();
        let mut cells = std::collections::HashSet::new();
        let mut per_room = std::collections::HashMap::new();
        for e in &d.events {
            assert!(cells.insert((e.x, e.y)), "two events on one cell");
            assert_ne!((e.x, e.y), d.start);
            assert_ne!((e.x, e.y), d.exit);
            let cell = d.cell(e.x as i64, e.y as i64);
            assert!(cell == FLOOR || cell == CORRIDOR, "event on a {cell}");
            assert_eq!(e.distance, d.distance(e.x as i64, e.y as i64));
            match e.kind.as_str() {
                "enemy" => {
                    assert!(e.distance >= 6);
                    let r = e.room.expect("room event");
                    assert_ne!(r, d.start_room);
                    *per_room.entry(r).or_insert(0) += 1;
                }
                "chest" => assert!(d.rooms[e.room.unwrap()].dead_end),
                "boss" => {
                    assert_eq!(e.room, Some(d.exit_room));
                    // the B marker of the throne template
                    assert!(d.rooms[d.exit_room].markers.iter().any(|m| m.2 == 'B' && (m.0, m.1) == (e.x, e.y)));
                }
                "shop" | "well" => assert!(!d.rooms[e.room.unwrap()].on_main_path),
                "trap" => {
                    assert_eq!(cell, CORRIDOR);
                    for o in d.events.iter().filter(|o| o.kind == "trap" && (o.x, o.y) != (e.x, e.y)) {
                        assert!(o.x.abs_diff(e.x).max(o.y.abs_diff(e.y)) >= 3);
                    }
                }
                other => panic!("unexpected event {other}"),
            }
        }
        // one enemy per room (per_room defaults to 1 for room rules)
        assert!(per_room.values().all(|&n| n == 1));
        assert_eq!(d.events.iter().filter(|e| e.kind == "boss").count(), 1);
        let enemies = d.events.iter().filter(|e| e.kind == "enemy").count();
        assert!(enemies <= 8);
        if !d.warnings.iter().any(|w| w.contains("(enemy)")) {
            assert!(enemies >= 5);
        }
    }
}

#[test]
fn config_errors_and_warnings_are_readable() {
    let ctx = ParseCtx { base: None, room_loader: None };
    let mut w = vec![];
    let e = Config::parse(&json!({ "events": [{ "type": "x", "where": "cielo" }] }), &ctx, &mut w).unwrap_err();
    assert!(e.contains("where") && e.contains("cielo"), "{e}");
    let e = Config::parse(&json!({ "templates": [{ "rows": ["###"] }] }), &ctx, &mut w).unwrap_err();
    assert!(e.contains("suelo"), "{e}");
    let mut w = vec![];
    Config::parse(&json!({ "widht": 30 }), &ctx, &mut w).unwrap();
    assert!(w[0].contains("widht"));
    // too many rooms for the map: generated with a warning, never an infinite loop
    let c = cfg(json!({ "width": 16, "height": 16, "rooms": [20, 20], "attempts": 3 }));
    let d = dg::generate(&c, 1).unwrap();
    assert!(d.rooms.len() < 20 && !d.rooms.is_empty());
    assert!(d.warnings.iter().any(|w| w.contains("salas")));
}

#[test]
fn generator_warnings_are_not_runtime_errors() {
    // a rule that cannot place all its events and an unknown key: printed once as warnings,
    // but `modengine check` must not count them as runtime errors (exit code 1)
    let dir = project(&[]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        global.d = dungeon_create({ width: 24, height: 24, rooms: 3, bogus_key: 1, events: [{ type: "chest", count: 40, where: "dead_end_cell" }] })
        dungeon_generate(global.d, 5)
        global.w = len(dungeon_info(global.d).warnings)
        "#,
    );
    assert!(num(&e, "w") >= 1.0);
    assert_eq!(e.error_count(), 0, "{:?}", e.errors);
    assert!(e.errors.is_empty());
}

#[test]
fn corridor_modes_and_single_room() {
    for mode in ["astar", "l", "wiggly"] {
        let c = cfg(json!({ "width": 48, "height": 48, "rooms": [6, 8], "corridor": mode, "doors": false }));
        let d = dg::generate(&c, 7).unwrap();
        assert!(!d.cells.contains(&DOOR));
        for (i, &cell) in d.cells.iter().enumerate() {
            if cell != WALL {
                assert!(d.dist[i] >= 0);
            }
        }
    }
    let c = cfg(json!({ "width": 20, "height": 20, "rooms": 1 }));
    let d = dg::generate(&c, 3).unwrap();
    assert_eq!(d.rooms.len(), 1);
    assert_ne!(d.start, d.exit);
    assert!(d.mst.is_empty());
}

#[test]
fn generation_is_fast() {
    let c = cfg(json!({ "width": 64, "height": 64, "rooms": [15, 15], "extra_edges": 0.2,
        "events": [{ "type": "enemy", "count": [6, 9], "where": "room" }, { "type": "chest", "count": 3, "where": "dead_end" }] }));
    // warm up, then the slowest of several seeds
    dg::generate(&c, 0).unwrap();
    let mut worst = 0.0f64;
    for seed in 1..=10 {
        let t = std::time::Instant::now();
        let d = dg::generate(&c, seed).unwrap();
        worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(d.rooms.len(), 15);
    }
    // debug builds are much slower: the bar is for release (cargo test --release)
    let limit = if cfg!(debug_assertions) { 200.0 } else { 20.0 };
    assert!(worst < limit, "64×64 with 15 rooms took {worst:.2} ms");
}

// ------------------------------------------------------------------ from scripts

const DUNGEON_SCRIPT: &str = r#"
global.cfg = {
    width: 48, height: 48, rooms: [6, 8], extra_edges: 0.3,
    templates: [
        { name: "hall", weight: 1, rows: ["......", ".E..E.", "......"] }
    ],
    rect_weight: 1,
    events: [
        { type: "enemy", count: [3, 5], where: "room", marker: "E" },
        { type: "chest", count: 1, where: "dead_end", data: { gold: 10 } }
    ]
}
"#;

#[test]
fn dungeon_functions_from_scripts() {
    let dir = project(&[("scripts/cfg.ms", DUNGEON_SCRIPT)]);
    let mut e = start(&dir);
    run(
        &mut e,
        r#"
        var d = dungeon_create(global.cfg)
        dungeon_generate(d, 77)
        var gr = dungeon_grid(d)
        global.gw = ds_grid_width(gr)
        var st = dungeon_start(d)
        var ex = dungeon_exit(d)
        global.start_cell = gr[# st.x, st.y]
        global.start_dist = dungeon_distance(d, st.x, st.y)
        global.exit_dist = dungeon_distance(d, ex.x, ex.y)
        global.path_len = len(dungeon_main_path(d))
        var rooms = dungeon_rooms(d)
        global.nrooms = len(rooms)
        global.start_room_flag = rooms[st.room].is_start
        global.events = dungeon_events(d)
        global.enemies = len(dungeon_events(d, "enemy"))
        var info = dungeon_info(d)
        global.hash1 = info.hash
        global.loops = info.loops
        var edges = dungeon_edges(d)
        global.mst_len = len(edges.mst)
        global.room_at = dungeon_room_at(d, st.x, st.y) == st.room
        global.cell = dungeon_cell(d, 0, 0)
        global.ascii = dungeon_ascii(d)
        dungeon_generate(d, 77)
        global.same_grid = dungeon_grid(d) == gr
        global.hash2 = dungeon_info(d).hash
        dungeon_generate(d, "otra semilla")
        global.hash3 = dungeon_info(d).hash
        // without a seed: from a stream (reproducible)
        var r = rng_create(9)
        dungeon_generate(d, null, r)
        global.hash4 = dungeon_info(d).hash
        rng_seed(r, 9)
        dungeon_generate(d, null, r)
        global.hash5 = dungeon_info(d).hash
        var dg = dungeon_distance_grid(d)
        global.dg_max = ds_grid_get_max(dg, 0, 0, 47, 47)
        dungeon_destroy(d)
        global.grid_freed = !ds_grid_exists(gr)
        global.gone = !dungeon_exists(d)
        "#,
    );
    assert_eq!(num(&e, "gw"), 48.0);
    assert_eq!(num(&e, "start_cell"), 0.0);
    assert_eq!(num(&e, "start_dist"), 0.0);
    assert_eq!(num(&e, "path_len"), num(&e, "exit_dist") + 1.0);
    assert!(num(&e, "nrooms") >= 6.0);
    assert!(g(&e, "start_room_flag").truthy());
    assert!(num(&e, "enemies") >= 3.0);
    assert_eq!(num(&e, "mst_len"), num(&e, "nrooms") - 1.0);
    assert!(g(&e, "room_at").truthy());
    assert_eq!(num(&e, "cell"), 1.0);
    assert!(g(&e, "ascii").to_string().contains('S'));
    assert!(g(&e, "same_grid").truthy());
    assert_eq!(g(&e, "hash1").to_string(), g(&e, "hash2").to_string());
    assert_ne!(g(&e, "hash1").to_string(), g(&e, "hash3").to_string());
    assert_eq!(g(&e, "hash4").to_string(), g(&e, "hash5").to_string());
    assert!(num(&e, "dg_max") > 0.0);
    assert!(g(&e, "grid_freed").truthy() && g(&e, "gone").truthy());
    // the chest carries its rule's data
    let Value::Array(evs) = g(&e, "events") else { panic!() };
    for ev in evs.borrow().iter() {
        let Value::Struct(s) = ev else { panic!() };
        let s = s.borrow();
        if s.fields[&intern("type")].to_string() == "chest" {
            let Value::Struct(data) = &s.fields[&intern("data")] else { panic!("no data") };
            assert_eq!(data.borrow().fields[&intern("gold")].as_num(), Some(10.0));
        }
    }
    assert!(e.errors.is_empty(), "{:?}", e.errors);
}

#[test]
fn debug_draw_batches_rectangles() {
    let dir = project(&[("scripts/cfg.ms", DUNGEON_SCRIPT)]);
    let mut e = start(&dir);
    run(&mut e, "global.d = dungeon_create(global.cfg)\ndungeon_generate(global.d, 3)");
    e.cmds.clear();
    run(&mut e, "dungeon_debug_draw(global.d, 0, 0, 4)");
    let n = e.cmds.len();
    // 48×48 cells merged in runs: far fewer commands than cells
    assert!(n > 50 && n < 1500, "{n} draw commands");
    e.cmds.clear();
    let only_ends = "{ cells: false, rooms: false, mst: false, extra: false, main_path: false, events: false }";
    run(&mut e, &format!("dungeon_debug_draw(global.d, 0, 0, 8, {only_ends})"));
    assert_eq!(e.cmds.len(), 4, "only start and exit, outlined");
    e.cmds.clear();
    // small cells (minimaps): whole cells, no outline
    run(&mut e, &format!("dungeon_debug_draw(global.d, 0, 0, 4, {only_ends})"));
    assert_eq!(e.cmds.len(), 2);
    assert!(e.run_code("dungeon_debug_draw(global.d, 0, 0, 4, { nope: true })").is_err());
}

#[test]
fn templates_from_files_and_room_assets() {
    let dir = project(&[
        ("dungeon/sala.txt", "#####\n#.E.#\n#...#\n#####\n"),
        ("objects/marca.ms", "object Marca { sprite = dot }\n"),
        ("sprites/muro.toml", "pixels = ['''\nwwww\nwwww\nwwww\nwwww\n''']\n"),
        (
            "rooms/plantilla.toml",
            "width = 40\nheight = 32\n\n[[layer]]\nname = \"walls\"\ntype = \"tiles\"\ntileset = \"muro\"\ncell = [8, 8]\nlegend = { \"#\" = 0 }\nmap = '''\n#####\n#..##\n#...#\n#####\n'''\n\n[[layer]]\ntype = \"instances\"\ninstances = [{ object = \"Marca\", x = 20, y = 20 }]\n",
        ),
    ]);
    let e = start(&dir);
    let mut w = vec![];
    let j = json!({ "rooms": [4, 4], "width": 40, "height": 40, "templates": [
        { "file": "dungeon/sala.txt", "name": "fichero" },
        { "room": "plantilla", "layer": "walls", "markers": { "Marca": "K" }, "name": "asset" }
    ]});
    let c = modengine::natives::dungeon::parse_config(&e, &j, &mut w).unwrap_or_else(|m| panic!("{m}"));
    assert_eq!(c.templates[0].rows[1].iter().collect::<String>(), "#.E.#");
    let rows: Vec<String> = c.templates[1].rows.iter().map(|r| r.iter().collect()).collect();
    assert_eq!(rows, vec!["#####", "#..##", "#.K.#", "#####"], "{w:?}");
}
