# Formato de proyecto

Todo son ficheros de texto (TOML y ModScript), fáciles de editar a mano y de versionar con git.
Los assets se identifican por el nombre del fichero sin extensión, y **todos comparten el mismo
espacio de nombres**: no puede haber un sprite `coin` y un sonido `coin`. Una convención útil es
`snd_` para sonidos y nombres en `PascalCase` para objetos.

## game.toml

```toml
[game]
name = "Coin Climber"
start_room = "title"                 # por defecto, la primera de `rooms`
rooms = ["title", "level1", "ending"]  # orden de next_room()/prev_room(); por defecto alfabético

[display]
width = 320          # resolución del juego (tamaño de la cámara)
height = 180
scale = 4            # tamaño inicial de la ventana = resolución × scale
pixel_perfect = true # escalar solo por números enteros (píxeles nítidos)
step_rate = 60       # pasos lógicos por segundo
letterbox = "#000000"
font_size = 16       # tamaño del texto por defecto
smooth = false       # filtrado suave en vez de píxeles nítidos
fullscreen = false

[palette]            # caracteres que pueden usar todos los sprites de pixel art
"k" = "#1a1c2c"
"r" = "#b13e53"
```

Para exportar (Archivo → Exportar…, ver [exportar.md](exportar.md)) hay además, todos opcionales:

```toml
[game]
version = "1.0.0"           # la que muestran el README y el APK
icon = "sprites/icon.png"   # PNG: icono de la ventana, del lanzador de Linux y del APK

[export]
exe_name = "CoinClimber"    # nombre del ejecutable (por defecto, el del juego)
out_dir = "export"          # carpeta de salida (+ /windows, /linux, /android)
windows_console = false     # true: el .exe abre también una consola (depurar)

[android]
package = "com.miestudio.coinclimber"  # por defecto com.modengine.<nombre>
version_code = 1                       # súbelo en cada versión que publiques
orientation = "landscape"              # landscape, portrait o sensor
virtual_pad = "arrows"                 # mando táctil: none, arrows (← →) o dpad
virtual_buttons = ["space:A", "enter:OK"]  # botones redondos: tecla[:etiqueta]
abis = ["arm64-v8a"]                   # y/o armeabi-v7a, x86_64 (emulador)
```

## Sprites (`sprites/`)

Un PNG suelto (`sprites/hero.png`) ya es un sprite de un frame con origen arriba a la izquierda.
Con un `.toml` del mismo nombre se describe mejor:

```toml
image = "hero_strip.png"
frames = 4               # tira horizontal de 4 frames...
# frame_size = [16, 16]  # ...o rejilla: corta la imagen en celdas (filas de izquierda a derecha)
origin = "center"        # [x, y] o "top_left", "top", "center", "left", "bottom"
fps = 10                 # velocidad de la animación con anim_speed = 1
mask = [4, 2, 11, 15]    # caja de colisión (izq, arriba, der, abajo) incluida. Por defecto: píxeles no transparentes
```

O dibujado directamente en el TOML. Cada carácter es un píxel según la paleta (la de
`game.toml` más la del propio sprite); `.` es transparente. Cada bloque `'''` es un frame:

```toml
origin = "center"
palette = { "g" = "#38b764" }
pixels = ['''
..gggg..
.gkggkg.
gggggggg
.g.gg.g.
''', '''
........
..gggg..
.gkggkg.
gggggggg
''']
```

### Paleta (`[palette]`) y Pear36

`[palette]` asigna **un carácter** a cada color; todos los sprites de pixel art pueden usarlo
(más los colores propios de cada sprite en `palette = {...}`, que tienen prioridad). `.` y el
espacio son transparentes.

Los proyectos nuevos (`modengine new` o «Crear un proyecto nuevo» en el editor) usan
**Pear36** (de PineappleOnPizza, [lospec.com/palette-list/pear36](https://lospec.com/palette-list/pear36))
con este mapeo estable: el color *i* de la paleta recibe el carácter *i* de
`0-9`, `A-Z`, `a-z`, `#@$%&*+=-~!?<>^/|:;` (81 como máximo; `'` no se usa nunca).

```toml
# Paleta: Pear36 de PineappleOnPizza (lospec.com). '.' es transparente.
[palette]
"0" = "#5e315b"   # 0-9: 5e315b 8c3f5d ba6156 f2a65e ffe478 cfff70 8fde5d 3ca370 3d6e70 323e4f
"A" = "#322947"   # A-J: 322947 473b78 4b5bab 4da6ff 66ffe3 ffffeb c2c2d1 7e7e8f 606070 43434f
"K" = "#272736"   # K-T: 272736 3e2347 57294b 964253 e36956 ffb570 ff9166 eb564b b0305c 73275c
# ...               U-Z: 422445 5a265e 80366b bd4882 ff6b97 ffb5b5
```

Los proyectos existentes no cambian: sus letras (`k`, `w`, `G`...) siguen funcionando igual.
Para pasar un proyecto a otra paleta está el diálogo **Paletas (Lospec)** del editor
(ver `docs/editor.md`): *añadir* los colores que falten (no toca ningún carácter existente; si
el carácter estándar de un color ya está ocupado recibe el primero libre después de los de la
paleta) o *reemplazar* `[palette]` y reasignar cada píxel de cada sprite al color más parecido
(distancia RGB ponderada «redmean»), quitando los `palette = {...}` propios de los sprites.
Antes de aplicar se copian `game.toml` y los sprites afectados a
`.modengine/palette_backup/<marca de tiempo>/` (con `files.txt`) para poder deshacerlo. Las
ediciones solo reescriben la tabla `[palette]` y la clave `pixels`: el resto de claves y los
comentarios se conservan.

Desde ModScript la paleta se lee con `palette_size()`, `palette_color(i | "c")`,
`palette_char(i)` y `palette_nearest(color)` (ver `docs/funciones.md`). El índice sigue el orden
de los caracteres de arriba, que para una paleta importada es su orden original.
Ejemplo: `examples/showcase_s5`.

## Sonidos (`sounds/`)

`.wav` y `.ogg` sueltos funcionan directamente. Con TOML:

```toml
file = "music.ogg"
volume = 0.6
```

o un efecto retro sintetizado, sin ficheros de audio:

```toml
[synth]
wave = "square"     # square, saw, triangle, sine, noise
freq = 280          # Hz al empezar
freq_end = 620      # Hz al terminar (deslizamiento)
duration = 0.12     # segundos
volume = 0.3
attack = 0.005
release = 0.05
duty = 0.5          # ancho de pulso (square)
vibrato = 0.0       # profundidad (fracción de la frecuencia)
vibrato_speed = 8
repeat = 1          # repetir el tono N veces...
repeat_pitch = 1.0  # ...multiplicando la frecuencia cada vez (1.26 = arpegio ascendente)
gap = 0.0           # silencio entre repeticiones
```

## Fuentes (`fonts/`)

Como en GameMaker, una fuente es una cara TTF/OTF «horneada» a un tamaño fijo: al cargar el
proyecto el motor rasteriza sus caracteres en una página de glifos (también sin ventana, así
`text_width` da medidas exactas en `modengine check`). Un `.ttf`/`.otf` suelto es una fuente de
tamaño 16 con los caracteres ASCII. Con TOML:

```toml
file = "pixel.ttf"            # junto a este fichero; sin `file` = la fuente integrada del motor
size = 8                      # píxeles
bold = false                  # negrita (sintetizada si la cara no es ya «Bold»)
italic = false                # cursiva (inclinada si la cara no es «Italic/Oblique»)
antialias = false             # false = glifos nítidos de pixel art (solo 0 o 255 de alfa)
ranges = [[32, 127], [160, 255]]   # caracteres incluidos (por defecto 32–127); 160–255 = ñ, á, ¿, ¡…
spacing = 0                   # píxeles extra entre caracteres
```

o una **fuente de sprite** (`font_add_sprite` de GameMaker): cada fotograma es un carácter.

```toml
sprite = "spr_digits"
chars = "0123456789"          # un carácter por fotograma, en orden
proportional = true           # recorta cada glifo a sus píxeles (false = ancho fijo)
spacing = 1
```

Los caracteres que no están en los rangos no se dibujan (como en GameMaker). Una cara que no se
puede leer no impide cargar el juego: se usa la fuente integrada y se avisa en la salida.

## Notas (`notes/`)

`notes/<nombre>.md`: texto Markdown para el equipo (tareas `- [ ]`, diseño, créditos). El juego
no las carga; el editor las muestra con vista previa.

## Tile sets (`tilesets/`)

Una imagen cortada en una rejilla de tiles, como el Tile Set de GameMaker. Las capas de tiles de
las rooms pintan con ellos.

```toml
sprite = "spr_world"          # un sprite del proyecto (se usa su hoja entera)...
# image = "world.png"         # ...o un PNG junto a este fichero
tile_width = 16
tile_height = 16
offset = [0, 0]               # píxeles antes del primer tile (borde de la hoja)
separation = [1, 1]           # píxeles entre tiles
fps = 5                       # velocidad de los tiles animados

[[animation]]                 # el tile 16 se dibuja como 16, 17, 18, 19, 16…
name = "agua"
frames = [16, 17, 18, 19]

[[autotile]]                  # auto tile de 16: el tile de cada celda según sus vecinos
name = "suelo"                # (arriba = 1, derecha = 2, abajo = 4, izquierda = 8)
tiles = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
```

Los tiles se numeran por filas desde 0. Un **dato de tile** (lo que guardan las capas y devuelve
`tilemap_get`) es el número de tile más bits de transformación como en GameMaker: espejo
`1 << 28` (268435456), volteo `1 << 29` y giro de 90° `1 << 30`. `-1` es una celda vacía
(en GameMaker el tile 0 es el vacío; aquí el 0 es un tile normal, igual que en las capas de
tiles antiguas). Ejemplo completo: `examples/showcase_assets_s1`.

## Rooms (`rooms/`)

```toml
width = 960               # opcional: por defecto cubre sus mapas, y como mínimo la pantalla
height = 208
background = "#41a6f6"
speed = 60                # pasos por segundo en esta room (opcional)
persistent = false        # true: al volver, todo sigue como se dejó
code = '''
print("room creada")      # ModScript que se ejecuta tras crear las instancias
'''

[camera]
follow = "Player"         # objeto (o nada) que sigue la cámara
smooth = 0.15             # 1 = pegada al objetivo; menos = más suave
x = 0                     # posición inicial
y = 0
```

### Capas

Las capas se dibujan por `depth`: mayor profundidad = más al fondo. Las instancias reciben la
profundidad de su capa (el objeto puede cambiarla con `depth = ...`).

```toml
[[layer]]                 # fondo: color y/o sprite
type = "background"
sprite = "hills"
color = "#000000"         # opcional, rellena la vista
tile_x = true             # repetir horizontalmente
tile_y = false
stretch = false           # estirar a toda la vista
parallax = [0.5, 1.0]     # 1 = se mueve con el mundo, 0 = fijo a la cámara
hspeed = -0.1             # desplazamiento automático
x = 0
y = 140
depth = 900
alpha = 1.0

[[layer]]                 # tiles con un tile set (tilesets/ts_world.toml)
name = "Mundo"
type = "tiles"
tileset = "ts_world"      # tile set... o un sprite cuyos frames son los tiles (proyectos antiguos)
cell = [16, 16]           # por defecto, el tamaño de tile
data = '''
.
6 14 14 12 . 268435478
3 11 11 9
'''                       # una línea por fila: datos de tile separados por espacios, `.` = vacío

[[layer]]                 # el formato de mapa también vale para tiles: cada carácter es un tile
type = "tiles"
tileset = "tiles"
legend = { "#" = 0, "=" = 3 }
map = '''
....====
########
'''

[[layer]]                 # instancias
type = "instances"
depth = 0
cell = [16, 16]
legend = { "#" = "Ground", "@" = "Player", "B" = ["Ground", "Coin"] }
map = '''
@.....
######
'''
instances = [
    { object = "Boss", x = 300, y = 64, vars = { hp = 20 } },
]
```

`vars` cambia las variables de esa instancia. Si el objeto les da tipo
(`hp: int(1, 50) = 5`), el valor se comprueba al cargar; los assets se escriben por nombre
(`drops = "Coin"`) y los colores como `"#rrggbb"`.

En los mapas, `.` y el espacio son celdas vacías. Las instancias de un mapa se colocan
centradas en su celda; si el sprite es más alto que la celda, se apoya en el borde inferior
(así una bandera de 24 px queda de pie sobre el suelo).

`tile_at(x, y)` y `tile_set(x, y, n)` leen y cambian tiles durante el juego; las funciones
`tilemap_*` de GameMaker (ver `docs/funciones.md`) trabajan con capas de tiles por nombre. El
editor guarda las capas que usan un tile set con `data` (admite cualquier número de tiles y los
bits de espejo/volteo/giro) y las que usan un sprite con `legend` + `map`, como antes.


## Trayectorias, líneas de tiempo, curvas y secuencias

Cuatro carpetas opcionales (un proyecto sin ellas carga igual). Cada fichero es un asset cuyo
nombre es el del fichero, accesible desde ModScript como cualquier otro asset. Ejemplo completo:
`examples/showcase_assets_s2`.

### Trayectorias (`paths/`)

Puntos que una instancia recorre con `path_start` (las *paths* de GameMaker).

```toml
closed = true      # el último punto se une con el primero
smooth = true      # esquinas redondeadas (curva B-spline cuadrática, como «Smooth curve» de GM)
precision = 4      # 1–8: subdivisiones de cada esquina curva (2^precision)
room = "level1"    # solo para el editor: room que se dibuja de fondo

[[point]]
x = 40
y = 40

[[point]]
x = 160
y = 24
speed = 50         # % de la velocidad de path_start en este punto (por defecto 100)
```

Con `smooth = true` la curva pasa por el punto medio de cada tramo y, si la trayectoria está
abierta, por el primer y el último punto (igual que GameMaker). La posición `0..1` se mide por
longitud real del recorrido.

### Líneas de tiempo (`timelines/`)

Código ModScript que se ejecuta en pasos concretos («momentos») de la timeline de una instancia,
con esa instancia como `self`.

```toml
[[moment]]
step = 0
code = "hspeed = 2"

[[moment]]
step = 60
code = '''
hspeed = -2
spawn(Spark, x, y)
'''
```

Un error de sintaxis en un momento se informa al cargar (`timelines/x.toml: moment 60: line 1: …`).

### Curvas de animación (`curves/`)

Canales con nombre; cada canal es una lista de puntos `[x, y]` con `x` entre 0 y 1.

```toml
[[channel]]
name = "y"
type = "smooth"            # linear | smooth (Catmull-Rom, pasa por los puntos) | bezier
points = [[0, 0], [0.5, 1], [1, 0]]

[[channel]]
name = "scale"
type = "bezier"
# en bezier cada punto puede llevar tiradores: [x, y, in_dx, in_dy, out_dx, out_dy]
points = [[0, 0, 0, 0, 0.42, 0], [1, 1, -0.42, 0, 0, 0]]
```

Fuera de `[0, 1]` el valor se queda en el del primer/último punto. Sin tiradores, los de bezier
son planos y a un tercio del tramo.

### Secuencias (`sequences/`)

Animaciones de pistas con fotogramas clave (las *sequences* de GameMaker).

```toml
length = 120          # duración en frames
fps = 30              # velocidad propia, independiente del step_rate de la room
playback = "loop"     # oneshot | loop | pingpong
origin = [0, 0]       # punto de la secuencia que se coloca en (x, y) al crearla
canvas = [320, 180]   # tamaño del lienzo del editor

[[track]]
name = "estrella"
type = "sprite"       # sprite | object | sound
asset = "star"
start = 0             # frames en los que existe la pista (end por defecto = length)
# end = 100

[[track.key]]
frame = 0
x = 160
y = 70
alpha = 0
ease = "ease_out"     # cómo se llega a la clave siguiente

[[track.key]]
frame = 40
angle = 360
scale_x = 2
scale_y = 2
alpha = 1

[[moment]]
frame = 40
code = "global.dings += 1"
```

- Cada clave solo fija las propiedades que escribe (`x`, `y`, `angle`, `scale_x`, `scale_y`,
  `alpha`, `image`); cada propiedad se interpola entre sus propias claves.
- `ease`: `linear` (por defecto), `smooth`, `step` (salto), `ease_in`, `ease_out`,
  `ease_in_out`, `bounce`, las familias de GameMaker `<familia>_<in|out|inout>` (`quad`,
  `cubic`, `quart`, `quint`, `sine`, `expo`, `circ`, `back`, `elastic`, `bounce`: p. ej.
  `elastic_out`, `back_inout`), o el **nombre de una curva de animación** (`"mi_curva"` usa su
  primer canal, `"mi_curva.canal"` uno concreto).
- `image`: frame del sprite. Si ninguna clave lo fija, el sprite se anima a su `fps`.
- Pistas `object`: crean una instancia del objeto mientras la pista está activa y la mueven
  cada paso (posición, ángulo, escala, alpha y frame); se destruye al salir de su rango o al
  destruir la secuencia.
- Pistas `sound`: el sonido suena cuando la cabeza pasa por `start` (la `alpha` de la primera
  clave, si existe, es el volumen).
- Los momentos ejecutan código global cuando la cabeza pasa por su frame.

## Sistemas de partículas (`particles/`)

Un fichero `particles/<nombre>.toml` describe un sistema de partículas al estilo de GameMaker:
**tipos** de partícula (cómo se ven y se mueven) y **emisores** (dónde y cuántas nacen). Se crea
en el juego con `part_system_create(<nombre>)`. Todas las claves son opcionales; los rangos se
escriben `[min, max]` o un solo número.

```toml
depth = -5                 # profundidad de dibujo (como las instancias)
new_below = false          # true: las partículas nuevas se dibujan debajo de las viejas

[[type]]
name = "flame"
shape = "flare"            # pixel disk square line star circle ring sphere flare spark explosion cloud smoke snow
# sprite = "spr_chispa"    # ...o un sprite (anula shape)
# animate = true           # reproduce la animación del sprite
# stretch = true           # la animación dura toda la vida de la partícula
# random_frame = true      # empieza en un frame al azar
size = [0.25, 0.45]        # 1 = 64 px con las formas integradas, 1 = tamaño real con sprites
size_incr = -0.006         # se suma en cada paso
size_wiggle = 0            # oscilación (no se acumula)
scale = [1, 1]             # escala x, y
speed = [0.4, 0.9]         # px por paso
speed_incr = 0
speed_wiggle = 0
direction = [80, 100]      # grados, 90 = arriba
direction_incr = 0
direction_wiggle = 6
gravity = 0.05             # px/paso² ...
gravity_dir = 270          # ...hacia abajo
orientation = [0, 360]     # giro del dibujo
orientation_incr = 0
orientation_wiggle = 0
orientation_relative = false   # suma la dirección de movimiento
colors = ["#ffe066", "#ff7a00", "#b13e53"]   # 1 a 3 colores a lo largo de la vida
color_mode = "fade"        # "mix": un color fijo al azar entre los dos primeros
alphas = [0.9, 0.7, 0.0]   # 1 a 3 valores a lo largo de la vida
additive = true            # mezcla aditiva (brillo)
life = [25, 40]            # pasos
step_type = "smoke"        # tipo creado en cada paso en cada partícula...
step_count = 0             # ...cuántas (negativo: una con probabilidad 1/n)
death_type = "smoke"       # tipo creado donde muere cada partícula
death_count = -3

[[emitter]]
name = "base"
type = "flame"             # nombre de un [[type]] de este fichero
shape = "ellipse"          # rectangle, ellipse, diamond o line
distribution = "gaussian"  # linear, gaussian (más en el centro) o invgaussian (más en los bordes)
region = [-5, -3, 5, 3]    # x1, y1, x2, y2 relativos a la posición del sistema
mode = "stream"            # stream: cada paso · burst: de golpe al crear el sistema
count = 2                  # por paso o por ráfaga (negativo en stream: una con probabilidad 1/n)
enabled = true
delay = [0, 0]             # pasos de espera antes de emitir
interval = [50, 70]        # ráfagas: se repiten cada tantos pasos (0 = solo una vez)
```

La simulación es la misma en el juego y en la vista previa del editor. El editor reescribe el
fichero al guardar (los comentarios se pierden).

## Shaders (`shaders/`)

Un shader son dos ficheros GLSL ES 1.00 con el mismo nombre, como en GameMaker:
`shaders/<nombre>.vsh` (vertex) y `shaders/<nombre>.fsh` (fragment). También vale la
estructura de GameMaker `shaders/<nombre>/<nombre>.vsh|fsh`. Si falta uno de los dos se usa
el de paso por defecto.

```glsl
// shaders/sh_gray.fsh
varying vec2 v_vTexcoord;
varying vec4 v_vColour;
uniform float u_strength;

void main() {
    vec4 c = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord);
    float g = dot(c.rgb, vec3(0.299, 0.587, 0.114));
    gl_FragColor = vec4(mix(c.rgb, vec3(g), u_strength), c.a);
}
```

Se escriben con los nombres de GameMaker: atributos `in_Position`, `in_Colour`,
`in_TextureCoord` (`in_Normal`), la matriz `gm_Matrices[MATRIX_WORLD_VIEW_PROJECTION]` (y
`MATRIX_VIEW`, `MATRIX_PROJECTION`, `MATRIX_WORLD`, `MATRIX_WORLD_VIEW`) y la textura
`gm_BaseTexture`. No hace falta `precision` ni `#version`. Los números de línea de los errores
son los del fichero.

Los shaders se compilan en la GPU al arrancar el juego (`modengine run`); si uno falla, el
error sale en la consola y en pantalla y se dibuja con el shader por defecto.
`modengine check` hace una comprobación estática (llaves, `main`, `gl_FragColor`/`gl_Position`,
sintaxis de GLSL 3 como `in`/`out`/`texture()`, `;` olvidados, varyings que no coinciden) y
`modengine shader <proyecto> [nombre] --check` los compila de verdad en la GPU.

## Extensiones (`extensions/`)

Una extensión es una carpeta `extensions/<nombre>/` con scripts ModScript (`.ms`, también en
subcarpetas) y un `extension.toml` opcional:

```toml
name = "easing"
version = "1.2.0"
author = "Ana"
description = "Funciones de easing"
enabled = true                       # false: no se carga nada de la carpeta
files = ["easing.ms", "tween.ms"]    # orden de carga (los que falten van después, por nombre)
init = "easing_init"                 # función llamada una vez al empezar el juego
final = "easing_final"               # función llamada al cerrar el juego

[constants]                          # nombres globales de solo lectura
EASING_VERSION = 120

[[function]]                         # API exportada: documentación + se comprueba que existe
name = "ease_out_back"
args = "t"
help = "pasa un poco del final y vuelve"
```

Los scripts de las extensiones se cargan **antes** que los del proyecto (su código suelto se
ejecuta primero y después `init`), y sus funciones y constantes son globales. Solo se admiten
extensiones ModScript: GameMaker también carga DLL/.so/JavaScript nativos, que aquí no existen
porque romperían la portabilidad y la seguridad del motor.
