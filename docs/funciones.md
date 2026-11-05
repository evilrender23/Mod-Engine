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
| `tile_at(x, y, [capa])`, `tile_set(x, y, n, [capa])` | leer/cambiar tiles (-1 = vacío) |

Colores: `rgb(r, g, b)` (0–255), `hsv(h, s, v)` (h 0–360, s y v 0–1), `color_mix(a, b, t)`,
`color_r(c)`, `color_g(c)`, `color_b(c)` y la tabla `color.white`, `color.red`, `color.gold`...

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
