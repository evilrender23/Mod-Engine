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

Un `.ttf` suelto (tamaño 16) o:

```toml
file = "pixel.ttf"
size = 8
```

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

[[layer]]                 # tiles decorativos: cada número es un frame del sprite `tileset`
type = "tiles"
tileset = "tiles"         # sprite cortado con frame_size
cell = [16, 16]
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

`tile_at(x, y)` y `tile_set(x, y, n)` leen y cambian tiles durante el juego.
