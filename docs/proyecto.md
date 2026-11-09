# Formato de proyecto

Todo son ficheros de texto (TOML y ModScript), fáciles de editar a mano y de versionar con git.
Los assets se identifican por el nombre del fichero sin extensión, y **todos comparten el mismo
espacio de nombres**: no puede haber un sprite `coin` y un sonido `coin`. Una convención útil es
`snd_` para sonidos y nombres en `PascalCase` para objetos. Si dos chocan, el proyecto no carga
y el error dice qué ficheros son:
`nombre de asset duplicado 'vestibulo': sprite `sprites/vestibulo.toml` y room `rooms/vestibulo.toml``
(para un objeto, el script y la línea donde se declara).

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
rotation = "nearest" # cómo se ven los sprites girados: nearest, rotsprite o smooth (ver «Rotación»)
rotation_steps = 0   # ángulos por vuelta de los sprites girados (0 = libre)
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
# internet = true                      # permiso INTERNET; sin poner: solo si el juego usa la red
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
rotation = "rotsprite"   # cómo se ve girado: nearest, rotsprite o smooth (por defecto, el de game.toml)
rotation_steps = 16      # ángulos por vuelta (0 = libre); por defecto, el de game.toml
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

### Pixel art compacto (`pixels_rle`) y `sprite-bake`

Un sprite grande escrito con `pixels` ocupa un carácter por píxel (un suelo de 288×288 son
más de 80 KB de TOML). `pixels_rle` es la misma imagen por tramos, con la misma paleta:

```toml
origin = "center"
pixels_rle = ['''
.*4R*8.*4
.*2R*12.*2 *3
0*3V*6U*3 7
''']
```

- `C*N` son N píxeles del carácter `C` (`R*8` = `RRRRRRRR`). Un carácter suelto es un píxel,
  así que una fila normal también es RLE válido.
- Una fila que acaba en un espacio y `*N` se repite N veces (` *3` = la fila y dos copias).
- Los espacios no son píxeles (el transparente es `.`): solo separan un tramo de un color que
  sea un dígito (`R*8 3` = ocho `R` y un `3`) y la fila de su repetición.
- El carácter `*` como color se escribe siempre con cuenta: `**1`, `**5`.
- `pixels` y `pixels_rle` no pueden ir juntos en el mismo sprite. Se cargan igual: mismo
  tamaño de frame, origen, máscara y `fps`.

El editor de sprites abre los dos formatos; la casilla **guardar compacto (pixels_rle)** del
panel de origen/máscara elige cómo se guarda.

Para pasar sprites de texto grandes a PNG (o a `pixels_rle`) desde la terminal:

```sh
modengine sprite-bake <proyecto> --dry-run       # qué cambiaría y cuánto ocupa antes/después
modengine sprite-bake <proyecto>                 # todos los sprites de texto de 8 KB o más
modengine sprite-bake <proyecto> suelo ruleta    # solo esos
modengine sprite-bake <proyecto> --rle           # a pixels_rle en vez de PNG
modengine sprite-bake <proyecto> --min-kb 2
```

Cada sprite queda como `sprites/<nombre>.png` (una tira con sus frames) y un TOML corto con
`image` y `frames` que conserva el resto de claves (`origin`, `mask`, `fps`, `rotation`...). Si
ya existe un PNG con ese nombre, el sprite no se toca. Los comentarios del TOML se pierden.

### Revisar pixel art (`modengine pixel-check`)

El motor rellena en silencio las filas cortas de `pixels`, así que un carácter de menos desplaza
el dibujo sin dar error. `modengine pixel-check <proyecto> <sprite>... | --all` comprueba el ancho
de cada fila, los caracteres que no están en la paleta (con frame, fila y columna), los colores
fuera de la paleta, el alfa, el origen y la línea de apoyo de las animaciones y, con `--tile`, las
uniones de un tile; `--preview DIR` genera vistas ampliadas sobre fondo claro y oscuro, el bucle y
la prueba 3×3. Detalles y la guía para dibujar en [pixel-art.md](pixel-art.md).

### Rotación de pixel art (`rotation`)

Girar pixel art con el muestreo «nearest» de la GPU deja bordes dentados y píxeles sueltos,
sobre todo en sprites grandes o escalados. Cada sprite elige cómo se ve girado (`rotation` en
su TOML, o el valor por defecto de `[display]`):

| `rotation` | Cómo se ve | Coste |
|---|---|---|
| `"nearest"` (por defecto) | la GPU gira la textura sin suavizar: bordes dentados | ninguno |
| `"rotsprite"` | RotSprite: el frame se amplía 8× con Scale2x, se gira y se reduce eligiendo el color más frecuente de cada píxel. Líneas limpias, sin colores nuevos | una imagen por (frame, ángulo), calculada la primera vez y guardada |
| `"smooth"` | filtrado lineal para ese sprite: suave pero borroso | ninguno |

- Con `rotsprite`, los ángulos se agrupan de 1° en 1° (o en los pasos de `rotation_steps`).
  Los múltiplos de 90° y los sprites sin girar se dibujan como siempre (son exactos).
  Funciona con cualquier escala uniforme (`image_xscale == image_yscale`, también negativa
  para espejar); con escalas distintas en x e y se usa la rotación normal.
- Cada ángulo nuevo se calcula una vez (unos milisegundos en un sprite de 64×64). El motor
  dedica como mucho ~6 ms por fotograma a ángulos nuevos: los que no caben ese fotograma se
  dibujan con la rotación normal y se calculan en los siguientes. Se guardan hasta ~96 MB de
  imágenes giradas; las que hace más tiempo que no se usan se descartan.
- `rotation_steps = N` redondea el ángulo a N pasos por vuelta en cualquier modo (giro «por
  pasos», como en muchos juegos de pixel art): `16` = de 22,5° en 22,5°.
- Con un shader activo, un sprite `rotsprite` girado usa la imagen ya girada como textura.

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

## Importar arte desde la terminal

`modengine import` copia PNG, WAV, OGG, TTF y OTF de cualquier carpeta al proyecto y escribe sus
`.toml`; es el mismo código que el diálogo **Importar assets** del editor y lo que usa el agente del
editor. `modengine inspect-image` describe imágenes para decidir cómo cortarlas.

```sh
modengine inspect-image ~/Descargas/pack --json          # tamaño, colores, dibujos, cortes probables
modengine import mijuego ~/Descargas/pack --dry-run      # qué haría, sin escribir nada
modengine import mijuego ~/Descargas/pack/hero.png --name hero_run --frames 6 --origin bottom-center
modengine import mijuego cave.png --kind tileset --tile 16x16 --offset 1,1 --sep 1,1
modengine import mijuego slime.png --grid 32x32 --row-names idle,walk,-,die   # un sprite por fila
modengine import mijuego atlas.png --crop 0,200,192,48 --frames 6 --name arbol
modengine import mijuego ~/Descargas/pack --credit "Kenney (kenney.nl), CC0" --json
```

- Una **carpeta** se recorre entera (sin `.DS_Store`, `Thumbs.db`, `__MACOSX` ni ocultos); lo que
  no es importable sale en *Omitidos* y los `LICENSE`/`README`/`credits` se listan aparte.
- **Nombres:** los del fichero, limpios (`Hero Run.png` → `hero_run`) y sin repetir (`coin_2`);
  `--name` para un fichero, `--prefix` añade la subcarpeta (`enemies/bat.png` → `enemies_bat`) y
  `--name-prefix spr_` un prefijo. Un `--name` ocupado es un error salvo con `--overwrite`, que
  manda el asset anterior a `.trash/`.
- **Frames** sin opciones, por el nombre: `hero_strip4.png` (tira de 4), `bat_16x16.png` (celdas de
  16×16), `coin_8.png` (8 frames cuadrados). Si no, un frame; cuando la imagen parece una hoja, la
  salida sugiere las opciones (`--frames 3`). Opciones: `--frames N`, `--grid WxH`, `--split-rows`
  / `--row-names a,b,-` (cada fila de la rejilla, un sprite), `--crop X,Y,W,H`, `--origin
  top-left|center|bottom-center|top|left|x,y`, `--fps F`.
- **Tile sets:** las imágenes con `tiles`, `tileset`, `terrain` o `ts_` en el nombre, `--kind
  tileset` o `--tile WxH`: la hoja se copia como el sprite `spr_<nombre>` y se escribe
  `tilesets/ts_<nombre>.toml` (`tile_width`, `tile_height`, `offset`, `separation`). Sin `--tile`, el
  tamaño sale del nombre (`_16x16`) o es 16, 32 u 8 si divide la imagen.
- `--credit "texto"` apunta los ficheros en `CREDITS.md`, en una sección por carpeta de origen.
- `--json` da `assets` (tipo, nombre, ficheros, frames, tamaño de frame, origen…), `skipped`,
  `license_files`, `credits` y `errors`. El código de salida es 0 si todo fue bien, 1 si algo
  falló (si el plan tiene errores no se escribe nada) y 2 si las opciones están mal.

## La Tienda desde la terminal (`modengine store`)

`modengine store` busca, enseña, descarga e importa assets gratuitos de
[OpenGameArt.org](https://opengameart.org): es el código de la
[Tienda del editor](editor.md#tienda-de-assets-opengameart) sin ventana, y lo que usa el
[agente](editor.md#assets-de-la-tienda-con-el-agente).

```sh
modengine store search slime --licencia cc0 --json         # resultados: slug, título, miniatura
modengine store search jump --tipo sonido --orden relevancia --pagina 2
modengine store show pixel-art-animated-slime --zip        # autor, licencias, ficheros y qué hay en el zip
modengine store download animated-slime                    # a la caché, para mirarla (inspect-image)
modengine store get animated-slime mijuego --frames 4 --origin bottom-center --name slime
modengine store get pixel-art-animated-slime mijuego --zip-entry slime-Sheet.png \
    --grid 32x25 --row-names idle,move,attack,hurt,die --name slime
modengine store get https://opengameart.org/content/<asset> mijuego --all --dry-run
```

- `search <palabras>`: `--tipo 2d|texturas|sonido|musica|fuentes` (por defecto 2d), `--licencia`
  con una lista (`cc0`, `cc-by`, `cc-by-4.0`, `oga-by`, `cc-by-sa`, `gpl`, `libres` —CC0, CC-BY y
  OGA-BY, por defecto—, `todas`), `--orden favoritos|recientes|relevancia`, `--pagina N`. Las
  licencias de cada resultado están en su ficha; `--json` las da solo si la ficha ya está en caché.
- `show <slug|url>`: título, autor, fecha, etiquetas, licencias con lo que exigen (crédito,
  compartir igual, GPL), aviso del autor, descripción, vistas previas y ficheros (tamaño y si son
  importables). `--zip` descarga los zip y lista su contenido. Termina con la orden `store get`
  sugerida.
- `download <slug|url> [fichero] [--zip-entry RUTA] [--out DIR]`: descarga sin importar (a la caché o
  a `DIR`) y dice dónde quedó, para verla o pasarle `modengine inspect-image`.
- `get <slug|url> <proyecto>`: `--file NOMBRE` (o su número en `show`) si el asset tiene varios
  ficheros, `--all` para todos los importables, `--zip-entry RUTA` (ruta, nombre o comodín:
  `'Slime/*.png'`) para entradas de un zip, y las opciones de [`modengine
  import`](#importar-arte-desde-la-terminal) (`--name`, `--frames`, `--grid`, `--origin`, `--fps`,
  `--tile`, `--split-rows`, `--row-names`, `--crop`, `--name-prefix`, `--overwrite`). Importa y añade
  al **`CREDITS.md`** del proyecto la sección del asset (autor, licencias, URL, ficheros, fecha y
  `<!-- oga:<slug> -->`); repetirlo no duplica nada. Los `LICENSE`/`README` del zip se extraen a la
  caché y se listan.
- **Licencias:** CC0 y las que solo piden crédito (CC-BY, OGA-BY) se importan escribiendo el
  crédito. Si todas son «compartir igual» (CC-BY-SA, GPL, LGPL) o no se reconocen, hace falta
  `--acepto-licencia`; sin él no se descarga nada, se explica lo que implican y el código de salida
  es **4**.
- **Red:** solo pide páginas `/content/`, búsquedas `/art-search-advanced` y ficheros de
  `/sites/default/files/` (lo que permite el `robots.txt`; otras direcciones se rechazan), con el
  `User-Agent` del editor, como mucho **una petición cada 2 s** —compartido con el editor y con
  otros `modengine store` que corran a la vez— y caché de 24 h en `~/.cache/modengine/store`
  (`$XDG_CACHE_HOME`). Si tiene que esperar turno, lo dice en stderr. `--json` incluye
  `network_requests`: cuántas peticiones fueron de verdad a la red.
- Códigos de salida: 0 bien, 1 error (red, página, importación), 2 opciones mal o hay que elegir
  fichero/entrada (la lista sale en el mensaje), 4 licencia no aceptada.
- Tests: `MODENGINE_STORE_FIXTURES=<carpeta>` hace que lea páginas de esa carpeta en vez de la red
  (`search.html`, `<slug>.html`, `files/…`) y apunte cada petición en `requests.log`; solo entonces
  `MODENGINE_STORE_INTERVAL_MS` puede acortar la espera.

## Sonidos (`sounds/`)

`.wav` y `.ogg` sueltos funcionan directamente. Con TOML:

```toml
file = "music.ogg"
volume = 0.6
```

Más campos (todos opcionales):

```toml
file = "boss_main.wav"
intro = "boss_intro.wav"   # suena una vez antes de `file`; luego solo `file` se repite
group = "musica"           # grupo de volumen (audio_group_set_gain); por defecto "default"
loop_start = 3.5           # segundos donde vuelve el bucle (por defecto: el final de la intro, o 0)
loop_end = 61.2            # segundos donde salta atrás (por defecto: el final)
```

`loop_start` y `loop_end` se miden desde el principio del sonido, intro incluida, y solo cuentan
cuando el sonido se toca en bucle (`play_sound(snd, true)`); sin bucle se oye entero una vez. Ver
[funciones.md](funciones.md#audio).

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

La fuente integrada (sin `file`) es ProggyClean: a 16 px (y 32, 48…) sus píxeles caen justos
y se ve nítida; a otros tamaños se rasteriza escalada (más suave). En los juegos de pixel art
(`smooth = false`) se dibuja sin filtrar y letra a letra sobre píxeles enteros.

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

[[layer]]                 # tiles de 16 px dibujados a 48 px: sin hornear un PNG más grande
type = "tiles"
tileset = "ts_casino"     # tile set de 16×16
scale = 3                 # o [3, 2]; celdas de 48×48 en la room
data = '''
0 1 1 2
'''

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

**Escala de una capa de tiles** (`scale = 3` o `scale = [x, y]`): los tiles se dibujan a esa
escala sin crear otra imagen, y la capa entera trabaja en píxeles escalados: la celda es
`cell × scale` (`cell` sigue siendo el tamaño del tile en el tile set), y las colisiones
(`tile_meeting`), `tile_at`, `tilemap_get_at_pixel`, `tilemap_get_tile_width`, el tamaño de
la room calculado a partir de las capas y la vista del editor usan ese tamaño. En el juego se
cambia con `tilemap_set_scale(capa, 3)` (ver `docs/funciones.md`); el editor la muestra en las
propiedades de la capa como **Escala**.

`tile_at(x, y)` y `tile_set(x, y, n)` leen y cambian tiles durante el juego; las funciones
`tilemap_*` de GameMaker (ver `docs/funciones.md`) trabajan con capas de tiles por nombre. El
editor guarda las capas que usan un tile set con `data` (admite cualquier número de tiles y los
bits de espejo/volteo/giro) y las que usan un sprite con `legend` + `map`, como antes.


## Prefabs (`prefabs/`)

Un prefab es un **grupo de instancias que se coloca de una vez**, como los prefabs de Unity o las
escenas empaquetadas de Godot: una plataforma con sus monedas, una torreta con dos escoltas, una
antorcha con su luz. Se define una vez y se reutiliza en cualquier room o desde un script; si
cambias el prefab, cambian todas las copias.

```toml
# prefabs/pf_torreta.toml
description = "Torreta con dos escoltas"
grid = [16, 16]               # solo para el editor

[[instance]]
object = "Turret"
x = 0                         # posiciones relativas al origen del prefab
y = 0
vars = { hp = 20 }            # como `vars` en una room: se comprueban los tipos

[[instance]]
object = "Guard"
x = -16
y = 0

[[instance]]
object = "pf_antorcha"        # otro prefab: se anida en esa posición
x = 0
y = -32
```

- Las rooms colocan un prefab **donde colocan un objeto**: en la leyenda de un mapa
  (`legend = { "T" = "pf_torreta" }`, con el origen en la esquina de arriba a la izquierda de la
  celda) o en `instances = [ { object = "pf_torreta", x = 100, y = 50, vars = { hp = 3 } } ]`.
  Los `vars` de la room se aplican a **todas** las instancias del grupo y ganan a los del prefab.
- **Anidar**: una instancia cuyo `object` es otro prefab se expande en su posición; sus `vars`
  ganan a las de dentro. Un prefab que se contiene a sí mismo (`a → b → a`) es un error al cargar.
- Todas las instancias de un grupo se crean antes de ejecutar ningún `create`, así que en el
  `create` de cualquiera ya existen las demás. Cada una recibe `prefab` (el asset) y
  `prefab_group` (array con todas las instancias del grupo); ver
  [funciones.md](funciones.md#prefabs).
- Los nombres comparten el espacio de nombres de los assets (una convención útil es `pf_`).
- Errores al cargar (objeto que no existe, variable fuera de rango, ciclo) dicen el fichero:
  `prefabs/pf_torreta.toml: hp: 99 is outside 1..50`.

Ejemplo: `examples/platformer/prefabs/pf_plataforma_monedas.toml`, colocado dos veces en el
mapa de `rooms/level1.toml` con el carácter `P`.

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

**Precisión.** El fragment shader usa `precision highp float` por defecto (en móviles sin highp
en fragment cae a `mediump`). Con `mediump` (±16384 garantizado) cualquier cuenta con
coordenadas de píxel desborda: `length(vec2(1280.0, 720.0))` eleva 1280² ≈ 1,6 millones y da
inf/NaN sin ningún error. Si un `.fsh` declara `precision mediump float` (o `lowp`) y hace
`length`/`distance`/`dot`/`pow` con `gl_FragCoord` o números ≥ 256, `modengine check` y
`modengine shader --check` lo avisan (`[aviso] shader '...': fsh:N: ...`), sin fallar.

**Mezcla.** Todos los dibujos, con shader o sin él, usan la misma mezcla: el color según el modo
(`gpu_set_blendmode`) y el alfa con `origen + destino·(1 − alfa_origen)`, así que el lienzo no
pierde opacidad con lo semitransparente, y se vuelca a la ventana opaco. Post-proceso de
pantalla completa: evento `post_draw` y surfaces (ver funciones.md, «Surfaces y post-proceso»).

Los shaders se compilan en la GPU al arrancar el juego (`modengine run`); si uno falla, el
error sale en la consola y en pantalla y se dibuja con el shader por defecto.
`modengine check` hace una comprobación estática (llaves, `main`, `gl_FragColor`/`gl_Position`,
sintaxis de GLSL 3 como `in`/`out`/`texture()`, `;` olvidados, varyings que no coinciden) y
`modengine shader <proyecto> [nombre] --check` los compila de verdad en la GPU.

## Tablas de datos (`data/`)

Contenido como datos: `data/<tabla>.toml` con registros `[[nombre]]` que llevan un `id` único
(también `.json`, una lista de objetos, o `.csv` con cabecera). `data/<tabla>.schema.toml`
(opcional) da el tipo de cada campo, valores por defecto, enums, rangos y referencias a assets
u otras tablas; los errores impiden cargar el juego y salen con `fichero:línea`. Los scripts los
leen con `data_get(tabla, id)`. Formato completo en [datos-y-cartas.md](datos-y-cartas.md).

```toml
# data/enemies.toml
[[enemy]]
id = "bat"
hp = 18
sprite = "spr_bat"
intents = [{ type = "attack", amount = 5 }]
```

## Idiomas (`lang/`)

Un fichero de textos por idioma, `lang/<código>.toml` (`es.toml`, `en.toml`, `pt-BR.toml`…):
`clave = "texto"` con `{marcadores}`; las tablas TOML dan claves con punto (`[menu]` +
`play = "Jugar"` es `menu.play`) y una tabla con `one`/`other` (y `zero`, `two`, `few`,
`many`) es un texto con plural. En `game.toml`:

```toml
[localization]
default = "es"    # idioma base y el del arranque (sin esto: el primero por orden alfabético)
fallback = "en"   # dónde buscar lo que falte en el idioma actual
```

Los scripts los leen con `loc("menu.play")`. Se editan como tabla en el editor (sección
Idiomas). Detalles en [juice-e-idiomas.md](juice-e-idiomas.md#idiomas).

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
