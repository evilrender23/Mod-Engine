# Exploración en primera persona por casillas

Cómo hacer un *dungeon crawler* de casillas ("blobber", como *Eye of the Beholder*, *Legend of
Grimrock* o *Vampire Crawlers*) con Mod Engine: el jugador avanza casilla a casilla, gira 90°,
y ve la mazmorra en 3D con los enemigos como sprites que siempre le miran. La referencia de
cada función está en [funciones.md](funciones.md#vista-en-primera-persona-fpview_).

La idea clave es separar **lógica** y **presentación**:

- la lógica (casilla `px, py` y dirección `dir`) cambia **al instante** y es lo que miran el
  combate, los eventos, el guardado y el minimapa;
- la cámara (`cx, cy, ca`) se **anima** hacia ese estado en unos pasos. Mientras se anima, las
  teclas se guardan en un *buffer* y se ejecutan al terminar, así que el control responde aunque
  el jugador pulse deprisa.

Moverse por casillas convierte la exploración en eventos discretos (paso, giro, choque,
entrar en una casilla) fáciles de guardar, repetir y enganchar a efectos. Prueba el movimiento
en un mapa pequeño antes de construir combate y menús.

## 1. La vista mínima

```
global.MAPA = [
    [1,1,1,1,1,1,1,1],
    [1,0,0,0,0,0,0,1],
    [1,0,1,1,1,1,0,1],
    [1,0,0,0,2,0,0,1],
    [1,1,1,1,1,1,1,1],
]

object Explorador {
    vista = -1

    on create {
        vista = fpview_create(640, 360)          // resolución interna de la imagen 3D
        fpview_set_map(vista, global.MAPA)        // mapa[y][x]: 0 vacío, n = tipo de pared
        fpview_set_wall(vista, 1, muro_piedra)    // sprites de 32×32 (u otro tamaño)
        fpview_set_wall(vista, 2, muro_musgo)
        fpview_set_floor(vista, suelo_losas)
        fpview_set_ceiling(vista, techo)          // o un color, o null (cielo = niebla)
        fpview_set_fog(vista, #120c1c, 1.5, 7.5)  // oscuridad desde 1.5 casillas, total a 7.5
        fpview_set_camera(vista, 1, 1, 0)         // casilla (1,1) mirando al este
    }

    on draw {
        fpview_draw(vista, 0, 0, 640, 360)
    }
}
```

`fpview_draw` dibuja donde estés dibujando: el fotograma, una surface (para encuadrarla en un
marco de UI, deformarla o aplicarle un shader) o dentro de `post_draw`. La cámara en `(x, y)`
está en el centro de la casilla; los ángulos son los de GameMaker (0 este, 90 norte) y las
direcciones de rejilla 0–3 (este, norte, oeste, sur) con `grid_dir_dx`, `grid_dir_dy`.

## 2. El bucle de movimiento

```
object Explorador {
    vista = -1
    px = 1          // estado lógico
    py = 1
    dir = 0
    cx = 1          // cámara animada
    cy = 1
    ca = 0
    de_x = 1
    de_y = 1
    de_a = 0
    a_x = 1
    a_y = 1
    a_a = 0
    t = 1           // 0..1 progreso de la animación (1 = parado)
    pasos = 1
    choque = false
    buffer = ""
    bloqueado = false   // p. ej. durante un combate

    // ... on create como arriba ...

    function animar(nx, ny, na, n, es_choque) {
        de_x = cx
        de_y = cy
        de_a = ca
        a_x = nx
        a_y = ny
        a_a = na
        t = 0
        pasos = n
        choque = es_choque
    }

    function accion(a) {
        if a == "izq" or a == "der" {
            dir = (dir + (a == "izq" ? 1 : 3)) mod 4
            // el ángulo no se normaliza: 270 → 360 gira por el camino corto
            animar(px, py, a_a + (a == "izq" ? 90 : -90), 12, false)
            exit
        }
        var d = dir
        if a == "atras" d = (dir + 2) mod 4
        if a == "lado_izq" d = (dir + 1) mod 4
        if a == "lado_der" d = (dir + 3) mod 4
        if fpview_can_move(vista, px, py, d) {
            px += grid_dir_dx(d)              // la lógica cambia ya
            py += grid_dir_dy(d)
            animar(px, py, a_a, 14, false)    // la cámara la sigue
        } else {
            // choque: asoma un quinto de casilla y vuelve
            animar(px + grid_dir_dx(d) * 0.2, py + grid_dir_dy(d) * 0.2, a_a, 10, true)
        }
    }

    on step {
        // buffer: la última tecla pulsada durante una animación se ejecuta al acabar
        if key_pressed(key.up) or key_pressed(key.w) buffer = "adelante"
        if key_pressed(key.down) or key_pressed(key.s) buffer = "atras"
        if key_pressed(key.left) buffer = "izq"
        if key_pressed(key.right) buffer = "der"
        if key_pressed(key.q) buffer = "lado_izq"
        if key_pressed(key.e) buffer = "lado_der"

        if t < 1 {
            t = min(1, t + 1 / pasos)
            var k = t * t * (3 - 2 * t)         // suavizado
            if choque k = sin(t * pi)           // ida y vuelta
            cx = lerp(de_x, a_x, k)
            cy = lerp(de_y, a_y, k)
            ca = lerp(de_a, a_a, k)
            if t >= 1 {
                if choque {
                    cx = de_x
                    cy = de_y
                } else if cx == px and cy == py {
                    al_entrar(px, py)
                }
            }
        } else if !bloqueado and buffer != "" {
            var a = buffer
            buffer = ""
            accion(a)
        } else if !bloqueado and key_down(key.up) {
            accion("adelante")                  // mantener la tecla sigue andando
        }
        // un leve balanceo al andar
        var h = 0.5
        if t < 1 and !choque and ca == a_a h = 0.5 + sin(t * pi) * 0.02
        fpview_set_camera(vista, cx, cy, ca, h)
    }

    function al_entrar(x, y) {
        // eventos de casilla: trampas, cofres, encuentros, escaleras...
        global.visitadas[y][x] = 1
        if x == 6 and y == 3 {
            bloqueado = true
            empezar_combate()
        }
    }
}
```

Variantes habituales:

- **Movimiento instantáneo** (opción de accesibilidad): `pasos = 1`.
- **Rotar el minimapa con el jugador** o no: es cosa del dibujo del minimapa, no de la lógica.
- **Girar hacia lo que choca**: en el choque, si delante hay un objeto rompible, gíralo hacia
  él (`dir = ...`) y ejecuta su efecto.
- **Pasos con efecto** (curar al andar, contar pasos del piso): en `al_entrar`.

## 3. Enemigos, objetos y apuntar

Los billboards se vuelven a poner en cada paso, así que salen directamente de tu estado:

```
fpview_clear_billboards(vista)
for (e in Enemigo) {
    var bob = sin(steps() * 0.1 + e.id) * 0.03
    e.bb = fpview_add_billboard(vista, e.cx, e.cy, e.sprite_index, e.image_index, 0.75 + bob)
}
```

Se ordenan por distancia y las paredes los tapan. `escala` es su alto en casillas (0.6 por
defecto); `altura` los hace flotar; `color` y `alfa` los tiñen (destello al recibir daño).

Para apuntar con el ratón: `fpview_pick(vista, mouse_x, mouse_y)` devuelve qué hay bajo el
puntero (los píxeles transparentes del sprite no cuentan, y una pared delante lo tapa):

```
if mouse_pressed(mouse.left) {
    var h = fpview_pick(vista, mouse_x, mouse_y)
    if h != null and h.kind == "billboard" {
        for (e in Enemigo) if e.bb == h.index e.recibir_golpe(3)
    }
}
```

Para poner un número de daño o un icono encima de un enemigo:

```
var p = fpview_project(vista, e.cx, e.cy, 0.9)     // 0.9 = casi a la altura del techo
if p != null draw_text(p.x, p.y, "-3")
```

## 4. Puertas, secretos y casillas especiales

- **Puertas y rejas** en el borde entre dos casillas: `fpview_set_edge(vista, x, y, dir, tipo)`
  con un tipo definido por `fpview_set_wall`. Para abrirla, `fpview_set_edge(..., 0)` o cambia a
  un tipo con `sólido = false` (se ve pero se atraviesa).
- **Muros secretos**: `fpview_set_cell(vista, x, y, 0)`.
- **Trampas, agujeros, alfombras**: `fpview_set_cell_floor(vista, x, y, sprite)`;
  `fpview_set_cell_ceiling` para claraboyas o techo roto (`null` = se ve el fondo).
- Si la mazmorra está en una `ds_grid` (por ejemplo, la de un generador procedural), pásala tal
  cual a `fpview_set_map`; se copia, así que después usa `fpview_set_cell` para los cambios.

## 5. Minimapa

```
// en create: una rejilla de visitadas del tamaño del mapa
global.visitadas = []
for (var y = 0; y < len(global.MAPA); y++) push(global.visitadas, array(len(global.MAPA[0]), 0))

// en draw_gui: ventana de 104×80 centrada en el jugador
fpview_draw_minimap(vista, 528, 8, 7, { width: 104, height: 80, visited: global.visitadas,
    background: #120c1c, unseen: #1c1726 })
```

Usa la cámara de la vista como jugador (con su ángulo, así que la flecha gira suave) y dibuja
los bordes (puertas). Con `draw_minimap(mapa, ...)` se dibuja cualquier array o ds_grid.

## 6. El estilo: pixelado, paleta y niebla

Para que la escena 3D case con sprites de pixel art, redúcela a una resolución fija con
posterizado, *dithering* y la paleta del juego:

```
on room_start {
    postfx_pixelate(320, 8, 0.5, true)   // 320 px de ancho, 8 niveles, dither 0.5, paleta de game.toml
}
```

Se aplica antes de `draw_gui`, así que el HUD y el texto siguen nítidos. Ajustes por zona: cada
room (o el objeto que la controla) llama a `postfx_pixelate` con sus valores en `room_start`;
`postfx_off()` lo quita. Para una transición de escena, sube `postfx_transition(t)` de 0 a 1,
cambia de room y bájalo otra vez.

Otros consejos:

- La **resolución interna** de `fpview_create` también pixela (320×180 dibujada a 640×360 da
  píxeles dobles sin post-proceso).
- La **niebla** oscura (`fpview_set_fog(v, #000000, 1, 6)`) da ambiente de mazmorra y oculta el
  final del mapa; un color claro da bruma exterior. El fondo (lo que no tiene techo) es del
  color de la niebla.
- `fpview_set_shading` oscurece las caras norte/sur: relieve sin luces.
- `fpview_set_fov(v, 70–90)`: más abierto se ve más pero deforma los bordes.

## 7. Rendimiento y pruebas

La vista se dibuja en la GPU con perspectiva y profundidad reales; el motor solo prepara las
caras cercanas (las demás se descartan por distancia y por estar detrás). Medido con un mapa de
64×64, 50 billboards, 640×360 y el post-proceso activo: ~0.1 ms de CPU y ~1.4 ms en total por
fotograma (más de 700 fps sin vsync).

Prueba el movimiento desde la terminal (ver [pruebas.md](pruebas.md)): pulsaciones con
`--input`, capturas a mitad de un giro con `--screenshot-at` y comprobaciones con `--assert`:

```sh
modengine run MiCrawler --no-save --input "10:up,30:up,50:right" \
    --screenshot-at 17,56 --screenshot out/paso_%04d.png --assert "80:Explorador.dir == 3"
```

Sin ventana (`modengine check`) no hay imagen, pero `fpview_can_move`, `fpview_pick` y
`fpview_project` funcionan igual, así que la lógica se puede probar entera.
