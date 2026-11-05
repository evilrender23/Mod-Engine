# Referencia de funciones

`modengine functions` lista todas las funciones integradas. Los argumentos entre corchetes son
opcionales. Los ángulos están en grados, con 0 = derecha y 90 = arriba (la Y de pantalla crece
hacia abajo). Los colores son números `0xRRGGBB`, que se escriben cómodamente como `#rrggbb`.

## Variables de instancia

| Variable | Significado |
|---|---|
| `x`, `y` | posición |
| `xprev`, `yprev` | posición al empezar el paso actual |
| `xstart`, `ystart` | posición donde se creó |
| `hspeed`, `vspeed` | velocidad horizontal/vertical (se suma a la posición en cada paso) |
| `speed`, `direction` | la misma velocidad en forma polar; cambiar una actualiza las otras |
| `gravity`, `gravity_dir` | aceleración añadida cada paso (dirección por defecto 270 = abajo) |
| `friction` | se resta a `speed` cada paso |
| `sprite`, `mask` | sprite que se dibuja / sprite cuya caja se usa para colisiones (`null` = el propio sprite) |
| `frame`, `frame_count` | frame actual (con decimales) / número de frames del sprite |
| `anim_speed` | multiplicador de la velocidad de animación (0 = parada) |
| `scale_x`, `scale_y` | escala; negativa = espejo |
| `angle` | rotación en grados (solo visual, la caja de colisión no rota) |
| `alpha`, `tint` | opacidad 0–1 / color que multiplica al sprite |
| `depth` | orden de dibujo: mayor = más al fondo |
| `visible` | si es `false` no se dibuja ni recibe eventos de dibujo |
| `solid` | ver "colisiones con sólidos" abajo |
| `persistent` | sobrevive al cambio de room |
| `alarm` | array de 12 contadores: `alarm[0] = 30` dispara `on alarm(0)` dentro de 30 pasos |
| `id`, `object` | la propia instancia / su objeto (solo lectura) |
| `bbox_left`, `bbox_right`, `bbox_top`, `bbox_bottom` | caja de colisión actual (solo lectura) |
| `sprite_width`, `sprite_height` | tamaño del sprite con la escala aplicada (solo lectura) |

Valores globales de solo lectura: `room`, `room_width`, `room_height`, `mouse_x`, `mouse_y`
(ratón en coordenadas de la room), `pi`, `infinity`, y las tablas `key`, `mouse`, `color`, `align`.

## Instancias

| Función | |
|---|---|
| `spawn(obj, x, y, [depth], [vars])` | crea una instancia y devuelve su id. `vars` es un struct de variables a asignar antes de `create`: `spawn(Bullet, x, y, 0, { speed: 6 })` |
| `destroy([objetivo], [run_event=true])` | destruye la instancia actual, una instancia, todas las de un objeto o `all` |
| `exists(objetivo)` | ¿existe esa instancia o alguna de ese objeto? |
| `count(obj)` | cuántas instancias hay (incluye hijos) |
| `find(obj, n)` | la n-ésima instancia, o `null` |
| `instances(obj)` | array con todas las instancias |
| `nearest(obj, [x, y])`, `furthest(obj, [x, y])` | la más cercana/lejana a un punto (por defecto, a quien llama) |
| `inst_at(x, y, obj)` | instancia cuya caja contiene el punto, o `null` |
| `is_a(inst_u_obj, obj)` | ¿es de ese objeto o de un hijo suyo? |
| `asset_name(asset_o_inst)` | nombre como texto |
| `distance_to(obj)` | distancia a la instancia más cercana de ese objeto |

## Colisiones

Las colisiones usan cajas alineadas con los ejes (la caja del sprite, o la de `mask`).

| Función | |
|---|---|
| `collides_at(x, y, obj)` | ¿chocaría con `obj` si estuviera en (x, y)? |
| `collider_at(x, y, obj)` | igual, pero devuelve la instancia (o `null`) |
| `colliders_at(x, y, obj)` | array con todas |
| `free_at(x, y)` | ¿no hay ninguna instancia `solid` ahí? |
| `rect_hits(x1, y1, x2, y2, obj)` | primera instancia que toca el rectángulo |
| `circle_hits(x, y, r, obj)` | primera instancia que toca el círculo |
| `line_hits(x1, y1, x2, y2, obj)` | primera instancia que cruza el segmento |
| `move_contact(dir, [max], [obj])` | avanza píxel a píxel en `dir` hasta tocar `obj` (o un sólido). Devuelve la distancia recorrida |

**Colisiones con sólidos:** en el evento `collision`, si la otra instancia es `solid`, la que
recibe el evento vuelve antes a su posición del paso anterior.

## Movimiento

| Función | |
|---|---|
| `motion_set(dir, speed)` | fija dirección y velocidad |
| `motion_add(dir, speed)` | suma un empujón a la velocidad actual |
| `move_towards(x, y, speed)` | apunta la velocidad hacia un punto |
| `step_towards(x, y, step)` | mueve directamente hacia un punto sin pasarse; devuelve `true` al llegar |
| `move_wrap([hor=true], [ver=true], [margen])` | al salir por un borde aparece por el contrario |

## Eventos y rooms

| Función | |
|---|---|
| `inherited()` | ejecuta la versión del padre del evento actual |
| `event_user(n, [objetivo])` | dispara `on user(n)` |
| `goto_room(room)` | cambia de room al terminar el paso |
| `next_room()`, `prev_room()` | según el orden de `game.toml`; devuelven `false` si no hay más |
| `restart_room()`, `restart_game()`, `quit()` | |
| `room_name()` | nombre de la room actual |

## Cámara

`camera_set(x, y)` (deja de seguir), `camera_follow(objetivo, [suavizado])`,
`camera_shake(intensidad, [pasos=10])`, `camera_x()`, `camera_y()`, `camera_w()`, `camera_h()`.

## Entrada

| Función | |
|---|---|
| `key_down(k)`, `key_pressed(k)`, `key_released(k)` | estado de una tecla (`key.space`, `key.a`...) |
| `key_any()`, `key_any_pressed()` | ¿alguna tecla? |
| `input_axis(neg, pos)` | -1, 0 o 1. Acepta teclas o arrays de teclas: `input_axis([key.left, key.a], [key.right, key.d])` |
| `mouse_down(b)`, `mouse_pressed(b)`, `mouse_released(b)` | botones (`mouse.left`, `mouse.right`, `mouse.middle`) |
| `mouse_wheel()` | movimiento de la rueda este paso |
| `gui_mouse_x()`, `gui_mouse_y()` | ratón en coordenadas de pantalla (para `draw_gui`) |
| `touch_count()`, `touch_x(i)`, `touch_y(i)` | dedos en una pantalla táctil (Android), en coordenadas de pantalla; un solo dedo también mueve el ratón |

## Dibujo

Solo tiene efecto dentro de los eventos de dibujo. Los ajustes (`set_color`...) se reinician al
empezar cada frame.

| Función | |
|---|---|
| `draw_self()` | dibuja la instancia como lo haría el motor por defecto |
| `draw_sprite(spr, frame, x, y)` | |
| `draw_sprite_ext(spr, frame, x, y, sx, sy, angle, color, alpha)` | |
| `draw_sprite_stretched(spr, frame, x, y, w, h)` | |
| `draw_rect(x1, y1, x2, y2, [contorno])`, `draw_circle(x, y, r, [contorno])` | |
| `draw_line(x1, y1, x2, y2, [grosor])`, `draw_tri(x1, y1, x2, y2, x3, y3, [contorno])`, `draw_point(x, y)` | |
| `draw_text(x, y, texto)` | con el color, alpha, fuente y alineación actuales; `\n` = nueva línea |
| `draw_text_ext(x, y, texto, escala, [color], [alpha])` | |
| `draw_bar(x1, y1, x2, y2, fracción, [fondo], [relleno])` | barra de vida/progreso |
| `set_color(c)`, `get_color()`, `set_alpha(a)`, `get_alpha()` | |
| `set_font(fuente_o_null)`, `set_align(h, [v])` | `align.left/center/right`, `align.top/middle/bottom` |
| `text_width(t)`, `text_height(t)` | |
| `layer_visible(nombre, visible)` | muestra u oculta una capa de la room |
| `tile_at(x, y, [capa])`, `tile_set(x, y, n, [capa])` | leer/cambiar tiles (-1 = vacío); más en [Tile maps](#tile-maps-y-tile-sets) |

Colores: `rgb(r, g, b)` (0–255), `hsv(h, s, v)` (h 0–360, s y v 0–1), `color_mix(a, b, t)`,
`color_r(c)`, `color_g(c)`, `color_b(c)` y la tabla `color.white`, `color.red`, `color.gold`...

### Paleta del juego

La `[palette]` de game.toml (Pear36 en los proyectos nuevos). Los índices van en el orden de
los caracteres `0-9`, `A-Z`, `a-z`, símbolos (el orden original de una paleta importada).

| Función | |
|---|---|
| `palette_size()` | número de colores |
| `palette_color(i)`, `palette_color("c")` | color por índice o por su carácter de pixel art (error si no existe) |
| `palette_char(i)` | carácter del color `i` (`""` si no existe) |
| `palette_nearest(c)` | índice del color de la paleta más parecido a `c` (-1 si está vacía) |

```
set_color(palette_color("F"))                  // Pear36: #ffffeb
tint = palette_color(palette_nearest(#ff0000)) // el rojo más cercano de la paleta
```

## Fuentes y texto

Las fuentes de `fonts/` (ver `docs/proyecto.md`) se usan con `set_font` / `draw_set_font`. Las
medidas salen de la fuente horneada, igual en el juego y sin ventana.

| Función | |
|---|---|
| `draw_set_font(f)` (= `set_font`), `draw_get_font()` (= `get_font`) | fuente actual (`null` = la integrada) |
| `string_width(t)`, `string_height(t)` | = `text_width` / `text_height` con la fuente actual |
| `string_width_ext(t, sep, w)`, `string_height_ext(t, sep, w)` | medidas del texto partido a `w` píxeles; `sep` = separación entre líneas (-1 = la de la fuente). También `text_width_ext`, `text_height_ext` |
| `draw_text_wrap(x, y, t, sep, w)` | dibuja partiendo líneas a `w` píxeles (respeta `set_align`) |
| `text_wrap(t, w)` | array con las líneas en que se parte el texto |
| `font_get_name(f)`, `font_get_fontname(f)` | nombre del asset / fichero TTF o sprite de origen |
| `font_get_size([f])` | tamaño en píxeles (sin argumento: el de la fuente actual) |
| `font_get_bold(f)`, `font_get_italic(f)` | estilo |
| `font_get_first(f)`, `font_get_last(f)` | primer y último carácter de sus rangos |
| `font_has_char(f, "ñ")` | ¿ese carácter está horneado en la fuente? |
| `font_get_info(f)` | struct `{ name, fontname, size, bold, italic, antialias, ranges, line_height, ascent, glyphs }` |

## Tile maps y tile sets

Un *tile map* es una capa de tiles de la room. Las funciones aceptan su nombre (`"Mundo"`), el
id que devuelve `layer_tilemap_get_id` o `null` (la primera capa de tiles). Las celdas se cuentan
desde 0 a partir del desplazamiento de la capa. Un *dato de tile* es el número de tile más los
bits de espejo/volteo/giro; `-1` = vacío.

| Función | |
|---|---|
| `layer_tilemap_get_id(capa)` | id del tile map de esa capa, o -1 |
| `layer_tilemap_exists(capa)` | ¿hay una capa de tiles con ese nombre? |
| `layer_tilemap_create(nombre, x, y, tileset, ancho, alto, [depth])` | crea una capa de tiles vacía (ancho × alto celdas) y devuelve su id |
| `tilemap_get(tm, cx, cy)`, `tilemap_set(tm, dato, cx, cy)` | leer / poner el dato de una celda |
| `tilemap_get_at_pixel(tm, x, y)`, `tilemap_set_at_pixel(tm, dato, x, y)` | lo mismo con coordenadas de la room |
| `tilemap_get_cell_x_at_pixel(tm, x, y)`, `tilemap_get_cell_y_at_pixel(tm, x, y)` | celda bajo un punto |
| `tilemap_get_width(tm)`, `tilemap_get_height(tm)`, `tilemap_set_width(tm, n)`, `tilemap_set_height(tm, n)` | tamaño en celdas |
| `tilemap_get_tile_width(tm)`, `tilemap_get_tile_height(tm)` | tamaño de celda en píxeles |
| `tilemap_get_x(tm)`, `tilemap_get_y(tm)`, `tilemap_x(tm, x)`, `tilemap_y(tm, y)` | posición de la capa |
| `tilemap_get_tileset(tm)`, `tilemap_tileset(tm, ts)` | tile set de la capa / cambiarlo |
| `tilemap_clear(tm, [dato])` | vacía la capa (o la rellena con `dato`) |
| `tilemap_autotile(tm, auto, cx, cy, [poner=true])` | pinta o borra con el auto tile `auto` (nombre o número) y recoloca los vecinos |
| `tile_meeting(x, y, [tm])` | ¿la caja de quien llama tocaría algún tile si estuviera en (x, y)? (colisiones con el escenario) |
| `tile_get_index(d)`, `tile_set_index(d, n)` | número de tile de un dato / cambiarlo conservando los bits |
| `tile_get_empty(d)`, `tile_set_empty(d)` | ¿vacío? / el dato vacío (-1) |
| `tile_get_mirror(d)`, `tile_get_flip(d)`, `tile_get_rotate(d)` | bits de transformación |
| `tile_set_mirror(d, [b])`, `tile_set_flip(d, [b])`, `tile_set_rotate(d, [b])` | devuelven el dato con ese bit puesto (o quitado con `false`) |
| `tileset_get_info(ts)` | struct `{ name, tile_width, tile_height, tile_count, columns, rows, fps, animations, autotiles }` |
| `draw_tile(ts, dato, frame, x, y)` | dibuja un tile (frame de un tile animado; -1 = el actual) |
| `draw_tilemap(tm, x, y)` | dibuja una capa de tiles en otra posición (minimapas, previsualizaciones) |

```js
// romper el bloque que hay bajo el ratón y dejar hierba alrededor
var tm = layer_tilemap_get_id("Mundo")
var cx = tilemap_get_cell_x_at_pixel(tm, mouse_x, mouse_y)
var cy = tilemap_get_cell_y_at_pixel(tm, mouse_x, mouse_y)
if mouse_pressed(mouse.right) tilemap_autotile(tm, "suelo", cx, cy, false)
```

## Audio

`play_sound(snd, [bucle=false], [volumen=1])`, `stop_sound(snd)`, `stop_all_sounds()`,
`sound_volume(snd, v)`.

## Tiempo y estado

`steps()` (pasos desde que empezó el juego), `time()` (segundos de juego), `fps()`,
`step_rate()`, `set_step_rate(n)`, `debug_mode([activo])`.

## Guardar datos

`save_data(nombre, valor)` guarda números, textos, arrays y structs como JSON en
`<proyecto>/saves/<nombre>.json`; `load_data(nombre, [por_defecto])` los recupera.
`to_json(v)` y `from_json(texto)` convierten sin tocar el disco.

## Matemáticas

`abs sign round floor ceil frac sqrt sqr pow exp ln log2 log10`,
`sin cos tan asin acos atan atan2` (radianes), `dsin dcos` (grados), `deg rad`,
`min(...) max(...) clamp(v, lo, hi) lerp(a, b, t) wrap(v, lo, hi)`,
`approach(v, objetivo, paso)` (acerca sin pasarse),
`dist(x1, y1, x2, y2)`, `angle_to(x1, y1, x2, y2)`, `angle_diff(a, b)`,
`len_x(longitud, dir)`, `len_y(longitud, dir)`.

Aleatorio: `rand([max=1])`, `rand_range(a, b)`, `irand(a, b)` (enteros, ambos incluidos),
`chance(p)`, `choose(a, b, ...)`, `seed(n)`, `randomize()`.

## Texto

`str(v)`, `num(texto)` (`null` si no es un número), `len(v)`, `upper`, `lower`, `trim`,
`substr(t, inicio, [cantidad])`, `replace(t, buscar, poner)`, `split(t, sep)`, `join(array, [sep])`,
`starts_with`, `ends_with`, `contains(t, sub)`, `index_of(t, sub)`, `chr(n)`, `ord(t)`,
`format("{} de {}", a, b)`, `pad(valor, ancho, [relleno])` → `pad(42, 6, "0")` = `"000042"`.

## Arrays y structs

Arrays: `array([n], [relleno])`, `range([desde], hasta)`, `push(a, v...)`, `pop(a)`,
`insert(a, i, v)`, `remove(a, i)`, `clear(a)`, `contains(a, v)`, `index_of(a, v)`, `first(a)`,
`last(a)`, `reverse(a)`, `slice(a, [inicio], [fin])`, `copy(v)`, `sum(a)`, `shuffle(a)`,
`sort(a, [comparar])`, `map(a, f)`, `filter(a, f)`.

Structs: `keys(s)`, `has(s, clave)`, `get(s, clave, [defecto])`, `set(s, clave, v)`,
`remove_key(s, clave)`.

Tipos: `type_of(v)`, `is_num`, `is_str`, `is_array`, `is_struct`, `is_func`, `is_inst`, `is_null`.

Otros: `print(...)` (a la consola), `error(mensaje)`, `call(f, args...)`.


## Trayectorias

Variables de instancia (como en GameMaker): `path_index` (la trayectoria o `-1`, solo lectura),
`path_position` (0–1), `path_positionprevious`, `path_speed` (píxeles por paso; negativa = hacia
atrás), `path_scale`, `path_orientation` (grados, antihorario), `path_endaction`.
Mientras sigue una trayectoria, `direction` apunta hacia donde se mueve. Evento:
`on path_ended` (la trayectoria termina con `path_action.stop`).

| Función | |
|---|---|
| `path_start(path, velocidad, [fin=path_action.stop], [absoluta=false])` | recorre la trayectoria. Relativa: empieza donde está la instancia. Absoluta: usa las coordenadas de la trayectoria. `fin`: `path_action.stop`, `.restart` (vuelve al principio), `.continue` (sigue desde donde acabó), `.reverse` (ida y vuelta) |
| `path_end()` | deja de seguirla |
| `path_get_length(p)`, `path_get_number(p)` | longitud en píxeles / número de puntos |
| `path_get_x(p, pos)`, `path_get_y(p, pos)`, `path_get_speed(p, pos)` | punto en la posición 0–1 |
| `path_get_point_x(p, n)`, `path_get_point_y`, `path_get_point_speed` | datos del punto n |
| `path_get_closed(p)`, `path_get_kind(p)` (0 recta, 1 suave), `path_get_precision(p)`, `path_get_name(p)`, `path_exists(p)` | |
| `path_add()`, `path_duplicate(p)`, `path_delete(p)`, `path_assign(destino, origen)`, `path_append(destino, origen)` | crear y copiar trayectorias en tiempo de juego |
| `path_add_point(p, x, y, [vel=100])`, `path_insert_point(p, n, x, y, [vel])`, `path_change_point(p, n, x, y, [vel])`, `path_delete_point(p, n)`, `path_clear_points(p)` | editar puntos |
| `path_set_closed(p, b)`, `path_set_kind(p, 0/1)`, `path_set_precision(p, 1–8)` | |
| `path_reverse(p)`, `path_mirror(p)`, `path_flip(p)`, `path_shift(p, dx, dy)`, `path_rescale(p, sx, sy)`, `path_rotate(p, grados)` | transformar (alrededor del centro) |
| `draw_path(p, x, y, absoluta)` | dibuja la trayectoria con el color actual |

## Líneas de tiempo

Variables de instancia: `timeline_index` (timeline o `-1`), `timeline_position` (paso actual,
con decimales), `timeline_speed` (por defecto 1), `timeline_running` (por defecto `false`),
`timeline_loop`. Cada paso, después de `step_begin`, se ejecutan los momentos entre la posición
anterior y la nueva, en el contexto de la instancia. Con `timeline_loop`, al pasar el último
momento vuelve a 0.

```
on create {
    timeline_index = intro
    timeline_running = true
}
```

| Función | |
|---|---|
| `timeline_start(tl, [velocidad=1], [bucle=false])` | atajo: asigna la timeline, posición 0 y en marcha |
| `timeline_stop()` | la para y vuelve a 0 |
| `timeline_add()`, `timeline_delete(tl)`, `timeline_exists(tl)`, `timeline_get_name(tl)` | |
| `timeline_moment_add_script(tl, paso, función)` | añade una función a un momento |
| `timeline_moment_clear(tl, paso)`, `timeline_clear(tl)` | |
| `timeline_size(tl)`, `timeline_max_moment(tl)` | número de momentos / último paso |

## Curvas de animación

Como en GameMaker, una curva es un struct `{ name, channels }` y cada canal
`{ name, type, iterations, points: [{ posx, value }] }` (`type`: `"linear"`, `"smooth"` o
`"bezier"`; también 0, 1, 2).

| Función | |
|---|---|
| `animcurve_get(curva)` | struct con todos los canales |
| `animcurve_get_channel(curva, nombre_o_índice)` | struct del canal |
| `animcurve_channel_evaluate(canal, x)` | valor del canal en `x` (0–1) |
| `animcurve_evaluate(curva, canal, x)` | atajo rápido sin structs: `animcurve_evaluate(salto, "y", t)` |
| `animcurve_get_channel_index(curva, nombre)` | índice o `-1` |
| `animcurve_exists(curva)` | |
| `animcurve_create()`, `animcurve_channel_new()`, `animcurve_point_new()`, `animcurve_destroy(c)` | crear curvas en tiempo de juego (structs que se evalúan igual) |

```
t = (t + 1 / 60) mod 1
y = base - 40 * animcurve_evaluate(bounce, "y", t)
```

## Secuencias

Una secuencia en juego es un *elemento* (un número) creado en una capa de la room; se dibuja a
la profundidad de esa capa y avanza sola cada paso a su `fps`.

| Función | |
|---|---|
| `layer_sequence_create(capa, x, y, seq)` | la crea en la capa (nombre, o una profundidad) y devuelve el elemento |
| `sequence_create(seq, x, y, [depth=0])` | atajo sin capa |
| `layer_sequence_destroy(el)`, `layer_sequence_exists([capa], el)` | |
| `layer_sequence_play(el)`, `layer_sequence_pause(el)`, `layer_sequence_is_paused(el)`, `layer_sequence_is_finished(el)` | `play` en una secuencia terminada vuelve a empezar |
| `layer_sequence_headpos(el, frame)`, `layer_sequence_headdir(el, 1/-1)`, `layer_sequence_speedscale(el, k)` | cabeza de reproducción, sentido y velocidad |
| `layer_sequence_x(el, x)`, `_y`, `_angle`, `_xscale`, `_yscale` | mover, girar y escalar toda la secuencia |
| `layer_sequence_get_x(el)`, `_get_y`, `_get_angle`, `_get_xscale`, `_get_yscale`, `_get_headpos`, `_get_headdir`, `_get_speedscale`, `_get_length`, `_get_sequence` | |
| `layer_sequence_get_track_instance(el, pista)` | instancia creada por una pista `object` (o `null`) |
| `sequence_exists(seq)`, `sequence_get(seq)` | struct con `name`, `length`, `playback_speed`, `playback`, `xorigin`, `yorigin`, `tracks`, `moments` |

Constante nueva: `path_action` (`stop`, `restart`, `continue`, `reverse`). Tipos de variables:
`path`, `timeline`, `animcurve`, `sequence` (`ruta: path = patrol`).

## Partículas

Funciones con los nombres de GameMaker (`colour` también se acepta como `color`). Sistemas,
tipos y emisores son números. Las coordenadas de las partículas son relativas a la posición del
sistema (que se mueve, gira y tiñe entero con `part_system_position/angle/color`).

| Función | |
|---|---|
| `part_system_create([asset])` | crea un sistema vacío o a partir de `particles/<asset>.toml`; las ráfagas del asset se disparan al crearlo |
| `part_system_create_layer(capa, persistente, [asset])` | igual, a la profundidad de esa capa de la room |
| `part_system_destroy(ps)`, `part_system_exists(ps)`, `part_system_clear(ps)` | `clear` quita partículas y emisores |
| `part_system_position(ps, x, y)`, `part_system_angle(ps, grados)` | mueve / gira todo lo que dibuja |
| `part_system_color(ps, color, alpha)` | tinte y opacidad de todo el sistema |
| `part_system_depth(ps, d)`, `part_system_layer(ps, capa)` | orden de dibujo entre instancias y capas |
| `part_system_automatic_update(ps, b)`, `part_system_automatic_draw(ps, b)` | con `false`, se actualiza / dibuja a mano con `part_system_update(ps)` / `part_system_drawit(ps)` |
| `part_system_draw_order(ps, viejas_a_nuevas)` | |
| `part_system_get_info(ps)` | struct `{ name, x, y, depth, particle_count, emitters: [{ ind, name, enabled, type }] }` |
| `part_particles_create(ps, x, y, tipo, n)` | crea `n` partículas (negativo: una con probabilidad 1/n) |
| `part_particles_create_color(ps, x, y, tipo, color, n)` | con un color fijo |
| `part_particles_burst(ps, x, y, asset)` | dispara una vez todos los emisores de un asset en (x, y) |
| `part_particles_count(ps)`, `part_particles_clear(ps)` | |
| `part_type_create()`, `part_type_destroy(t)`, `part_type_exists(t)`, `part_type_clear(t)` | destruir un tipo borra sus partículas |
| `part_type_shape(t, pt_shape_*)` | `pt_shape_pixel, disk, square, line, star, circle, ring, sphere, flare, spark, explosion, cloud, smoke, snow` |
| `part_type_sprite(t, spr, animar, estirar, al_azar)` | |
| `part_type_size(t, min, max, incr, osc)`, `part_type_scale(t, xs, ys)` | tamaño 1 = 64 px con las formas |
| `part_type_speed(t, min, max, incr, osc)`, `part_type_direction(t, min, max, incr, osc)` | |
| `part_type_gravity(t, cantidad, dir)` | |
| `part_type_orientation(t, min, max, incr, osc, relativa)` | |
| `part_type_color1/2/3(t, c1, [c2], [c3])`, `part_type_color_mix(t, c1, c2)` | colores a lo largo de la vida / uno fijo al azar |
| `part_type_color_rgb(t, rmin, rmax, gmin, gmax, bmin, bmax)`, `part_type_color_hsv(...)` | color fijo al azar por canal (0–255) |
| `part_type_alpha1/2/3(t, a1, [a2], [a3])`, `part_type_blend(t, aditiva)` | |
| `part_type_life(t, min, max)` | en pasos |
| `part_type_step(t, n, otro_tipo)`, `part_type_death(t, n, otro_tipo)` | subpartículas en cada paso / al morir |
| `part_emitter_create(ps)`, `part_emitter_destroy(ps, em)`, `part_emitter_destroy_all(ps)`, `part_emitter_exists(ps, em)`, `part_emitter_clear(ps, em)` | |
| `part_emitter_region(ps, em, xmin, xmax, ymin, ymax, ps_shape_*, ps_distr_*)` | `ps_shape_rectangle/ellipse/diamond/line`, `ps_distr_linear/gaussian/invgaussian` |
| `part_emitter_stream(ps, em, tipo, n)`, `part_emitter_burst(ps, em, tipo, n)` | flujo continuo / ráfaga inmediata |
| `part_emitter_enable(ps, em, b)`, `part_emitter_interval(ps, em, min, max)`, `part_emitter_delay(ps, em, min, max)` | intervalo y retardo en pasos |
| `part_emitter_find(ps, nombre)` | índice del emisor de un asset por nombre (o -1); propio de Mod Engine |

```
fuego = part_system_create(ps_fire)
part_system_position(fuego, x, y)

chispa = part_type_create()
part_type_shape(chispa, pt_shape_spark)
part_type_size(chispa, 0.1, 0.2, -0.002, 0)
part_type_color2(chispa, #ffffff, #ffcd75)
part_type_alpha2(chispa, 1, 0)
part_type_blend(chispa, true)
part_particles_create(fuego, 0, 0, chispa, 10)
```

## Shaders y modos de mezcla

| Función | |
|---|---|
| `shader_set(sh)`, `shader_reset()`, `shader_current()` | lo que se dibuja después usa ese shader (cada frame empieza sin shader) |
| `shader_is_compiled(sh)` | `false` si la GPU lo rechazó (sin ventana: si la comprobación estática falla) |
| `shader_get_uniform(sh, "nombre")` | identificador del uniform, o -1 si no existe |
| `shader_set_uniform_f(u, v1, [v2], [v3], [v4])`, `shader_set_uniform_i(u, ...)` | tantos valores como el tipo (`float`, `vec2`…, `int`, `ivec2`…) |
| `shader_set_uniform_f_array(u, array)`, `shader_set_uniform_i_array(u, array)`, `shader_set_uniform_matrix(u, array16)` | arrays y `mat4` |
| `shader_get_sampler_index(sh, "nombre")`, `texture_set_stage(sampler, sprite)` | texturas extra (`uniform sampler2D`) |
| `gpu_set_blendmode(bm_normal / bm_add / bm_subtract / bm_max)`, `gpu_get_blendmode()` | modo de mezcla de lo que se dibuja después |

```
on draw {
    shader_set(sh_wave)
    shader_set_uniform_f(shader_get_uniform(sh_wave, "u_time"), time())
    draw_self()
    shader_reset()
}
```

## Extensiones

`extension_exists(nombre)` (¿está cargada y activa?) y `extension_get_info(nombre)` (struct
`{ name, version, author, description, enabled, files, functions }`). Las funciones y
constantes de las extensiones se usan como cualquier otra función global.
