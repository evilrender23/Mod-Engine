# Probar juegos desde la terminal

Guía para agentes (y personas) que desarrollan un juego con Mod Engine y necesitan
**verificarlo sin adivinar**: entrada simulada con teclado y ratón, capturas en pasos
lógicos exactos sin abrir ventanas en el escritorio, azar reproducible, código y
comprobaciones en pasos concretos, partidas guardadas aisladas, objetos de prueba y
perfil de rendimiento.

Todas las opciones valen para `modengine check` (sin ventana, lo más rápido),
`modengine run` (con GPU) y `modengine server` / `run --headless` salvo donde se indica.

```sh
modengine check MiJuego --steps 300 --seed 1 --no-save \
    --input "10:mouse=320,180, 20-120:mb_left" \
    --assert "120:count(Bala) > 0" --profile
modengine run MiJuego --screenshot-at 60,200,400 --screenshot capturas/s_%04d.png --seed 1 --no-save
```

## Pasos lógicos

El juego avanza en **pasos lógicos** (`step_rate` por segundo, 60 por defecto). Todas las
opciones de prueba cuentan pasos, no frames de pantalla: el paso 1 es el primer `step`
tras empezar la primera room; el paso 0 es justo después de empezar (antes del primer step).
`steps()` en ModScript devuelve el mismo número.

En `run` con `--screenshot-at` o `--steps`, el bucle va **a paso fijo**: exactamente un
paso lógico y un dibujado por frame, con vsync desactivado. El resultado no depende de la
velocidad del ordenador ni del refresco del monitor (`--frames` sí depende: evítalo).

## Códigos de salida

| código | significado |
|---|---|
| 0 | todo bien |
| 1 | errores de ejecución (o el proyecto no carga) |
| 2 | argumentos incorrectos (opción desconocida, `--assert` que no compila...) |
| 3 | falló algún `--assert` (también si además hubo errores de ejecución) |

`check`, `run` y `server` siguen estas reglas. Los errores se cuentan sin repetir (el mismo
mensaje en cada paso cuenta una vez): `N runtime error(s)` al final.

## `--input`: teclado, texto y ratón

Lista separada por comas de `PASO:ACCIÓN` o `DESDE-HASTA:ACCIÓN` (mantener desde el paso
DESDE hasta el anterior a HASTA; en HASTA se suelta).

| acción | qué hace |
|---|---|
| `30:space` | pulsa y suelta espacio en el paso 30 (`key_pressed` en 30) |
| `10-200:right` | mantiene la flecha derecha del paso 10 al 199 |
| `40:text=Ana` | escribe «Ana» en el paso 40 (`keyboard_string`; sin comas) |
| `10:mouse=320,200` | pone el puntero en (320, 200) **de pantalla/GUI** desde el paso 10 |
| `20-80:mouse=0,0>300,150` | mueve el puntero en línea recta de (0,0) a (300,150) entre los pasos 20 y 79 |
| `30:mouse_room=500,60` | puntero en coordenadas **de la room** (se resta la cámara tal como está al empezar cada paso; si la cámara se mueve durante el paso, `mouse_x` varía un poco) |
| `20-80:mb_left` | mantiene el botón izquierdo (también `mb_right`, `mb_middle`; alias `mouse_left`, `click`) |
| `25:mb_left` | un clic: pulsado en 25, soltado en 26 |
| `90:wheel_up` | una muesca de rueda hacia arriba (`wheel_down` hacia abajo); con rango, una por paso |

- El puntero se queda donde se dejó la última vez (como un ratón real). Antes del primer
  `mouse=` vale (0, 0) en `check`, o lo que diga el ratón real en `run`.
- Las coordenadas GUI son las de `mouse_x`/`mouse_y` en `draw_gui` (resolución del juego,
  sin escalar la ventana). Las de room son las de `mouse_x`/`mouse_y` en los eventos normales.
- Las posiciones llevan su propia coma (`mouse=320,200`); también vale `mouse=320;200`.
- Funciona igual sin ventana (`check`, `--headless`) y con ventana; con ventana los botones
  simulados se suman a los reales.

## Capturas en pasos exactos: `--screenshot-at`

```sh
modengine run MiJuego --screenshot-at 200,500,900                 # shot_0200.png, shot_0500.png, shot_0900.png
modengine run MiJuego --screenshot-at 60,120 --screenshot out/menu.png   # out/menu_0060.png, out/menu_0120.png
modengine run MiJuego --screenshot-at 100 --screenshot "s_%d.png"        # s_100.png ({step} también vale)
modengine run MiJuego --screenshot-at 100 --crop 0,0,160,90               # solo esa zona (píxeles del juego)
modengine check MiJuego --screenshot-at 100                               # lo mismo desde check
```

- La captura es la imagen final del juego **a su resolución** (`[display] width × height`,
  después de `post_draw` si lo hay), sin el escalado de la ventana, la barra de F3 ni la
  barra roja de errores. Es opaca, como se ve en la ventana.
- Se toma justo después de dibujar ese paso. El juego termina tras la última captura, o más
  tarde si `--steps N` o algún `--exec-at N:` / `--assert N:` nombra un paso posterior.
- Con `--screenshot-at` la ventana **no aparece en el escritorio** (ver abajo); `--show`
  la muestra. `--screenshot F --frames N` sigue existiendo, pero cuenta frames.
- `check --screenshot-at` y `check --render` usan la GPU igual que `run` (en ventana oculta);
  `check` sin ellas no dibuja en la GPU, solo ejecuta los eventos de dibujo.

### Ventana oculta (`--hidden`)

`--hidden` (implícito con `--screenshot-at` y `check --render`) abre la ventana de OpenGL y
la retira del escritorio en el primer frame:

- **Linux (X11 y Wayland vía XWayland)**: la ventana se retira (`XWithdrawWindow`); el
  juego sigue dibujando en su lienzo, que es lo que leen las capturas. Puede verse un
  parpadeo de un instante al arrancar. Sin servidor gráfico (SSH, CI) no hay OpenGL:
  usa `xvfb-run modengine run ...` o `kwin_wayland --virtual --xwayland -- modengine run ...`,
  o `check` sin `--render`.
- **Windows / macOS**: la ventana se mueve fuera de la pantalla (puede verse en la barra de
  tareas).

`--hidden` también vale sin capturas, p. ej. `run --hidden --steps 600 --assert ...` para
probar con la GPU real.

## `--seed N`: azar reproducible

Con `--seed N`, `rand`, `rand_range`, `irand`, `chance`, `choose`, `shuffle`, las partículas
y el temblor de cámara empiezan en N. `randomize()` deja de usar el reloj: cada llamada
elige una semilla derivada de N, así que un juego que llama a `randomize()` al empezar cada
nivel sigue generando niveles distintos entre sí, pero iguales en cada ejecución.

Sin `--seed` el motor ya parte de una semilla fija; lo que cambia de una ejecución a otra
es `randomize()`. `seed(n)` en el código sigue funcionando igual.

Comprobar que dos ejecuciones son idénticas:

```sh
modengine run J --seed 5 --no-save --screenshot-at 300 --screenshot /tmp/a.png
modengine run J --seed 5 --no-save --screenshot-at 300 --screenshot /tmp/b.png
cmp /tmp/a_0300.png /tmp/b_0300.png && echo idénticas
```

## `--exec-at PASO:CÓDIGO` y `--assert`

- `--exec-at 120:with Jugador vida = 1` ejecuta código (ámbito global) **después** del paso
  120, antes de dibujarlo (la captura de ese paso ya lo refleja). Se puede repetir. El paso 0
  es justo después de empezar la primera room (como `--exec`).
- `--assert EXPR` se comprueba **al final**; `--assert 300:EXPR` después del paso 300 (tras
  los `--exec-at` de ese paso); `--assert "*:EXPR"` después de **cada** paso (informa del
  primer fallo). Falla si el valor es falso o si da un error. Un `--assert N:` cuyo paso no se
  alcanzó (el juego terminó antes) también falla.
- EXPR es una **expresión** (no sentencias: nada de `with` ni `if`). Para leer una variable de
  un objeto usa `Objeto.var` (la primera instancia), `count(Objeto)`, `exists(Objeto)` o una
  función de un `--test-script` que haga el trabajo.
- Al final: `asserts: 3 de 3 correctos`, o los fallos en stderr con el paso, la room y el
  valor obtenido, y código de salida 3:

```
[assert] --assert fallido en el paso 300 (room 'nivel1'): count(Enemigo) == 0  (vale false)
```

Usa comillas simples en la shell para que `$`, `!` o `*` no se interpreten.

## Partidas guardadas: `--save-dir` y `--no-save`

`save_data`/`load_data` y las funciones de ficheros (`buffer_save`, `buffer_load`...)
escriben en la carpeta de guardado del juego (`<proyecto>/saves`, o la carpeta del usuario
en juegos exportados). En pruebas:

- `--save-dir DIR`: usa DIR (se crea si hace falta). Para probar con una partida concreta,
  copia antes sus ficheros a DIR.
- `--no-save`: una carpeta temporal vacía que se borra al terminar. `saves/` del proyecto
  ni se lee ni se toca. (Si el proceso se mata con una señal, la carpeta
  `modengine-nosave-*` puede quedarse en el directorio temporal.)

## `--test-script FICHERO.ms`: objetos de prueba

Carga ficheros ModScript extra **sin copiar el proyecto ni tocar las rooms** (se puede
repetir). Sus funciones y su código de nivel superior se cargan como los del proyecto (al
final). Cada objeto que definan se **crea solo en cada room** que no tenga ya una instancia
suya (en (0, 0), después de las instancias y el código de la room, antes de `room_start`).
Por eso un objeto de prueba `persistent = true` existe una sola vez aunque se cambie o se
reinicie la room. Los objetos cuyo nombre empieza por `_` no se crean solos (ayudantes).
Si el fichero está dentro de la carpeta del proyecto ya se carga con él (no se duplica);
lo mejor es tenerlo fuera, para que el juego normal no lo incluya.

```js
// prueba_disparo.ms
global.disparos = 0

object Sonda {
    persistent = true
    on step {
        global.disparos += count(Bala) > 0 ? 1 : 0
        if steps() == 200 print("balas:", count(Bala), "jugador en", Jugador.x, Jugador.y)
    }
}

function prueba_vida() { return Jugador.vida > 0 }
```

```sh
modengine check MiJuego --test-script prueba_disparo.ms --steps 240 \
    --exec-at '100:print(prueba_vida())' --assert 'global.disparos > 30'
```

## `--profile`

Al terminar imprime cuánto tardó cada evento de cada objeto (tiempo propio: lo que tarden
los eventos que dispare, p. ej. un `create` dentro de un `step`, se cuenta en el suyo), ordenado
de mayor a menor, y el total por objeto. Arriba, el tiempo de los pasos lógicos y del
dibujado (`render`, sin la GPU), el tiempo total desde el arranque (incluye crear la
primera room y, con ventana, la GPU) y qué parte es de los eventos; el resto es del motor
(movimiento, colisiones, partículas...). Vale en `check`, `run` y `server`.

```
=== perfil (--profile) ===
pasos lógicos: 240  (85.31 ms en total, 0.355 ms/paso)   dibujado: 240 (165.20 ms, 0.688 ms/frame)
tiempo total: 512.70 ms (desde el arranque: carga de la room, pasos, dibujado y, con ventana, la GPU)
en eventos: 492.41 ms (96% del total); el resto es el motor (...)

por evento (tiempo propio, sin los eventos que dispara):
objeto                   evento                      llamadas    total ms   media µs     máx µs      %
Planta                   create                             1     262.044  262043.91   262043.9   53.2
Partida                  draw_gui                         240     110.929     462.20     6005.6   22.5
Jugador                  step                             240      13.577      56.57     1924.3    2.8
...
```

## Recetas

**Probar disparo sostenido con el ratón** (apuntar y mantener el botón izquierdo de los pasos
60 a 179). Un `--test-script` cuenta las balas que aparecen (las balas viven poco: contar en
un paso suelto no basta):

```js
// sonda.ms
global.disparos = 0
object Sonda {
    persistent = true
    antes = 0
    on step_end {
        var n = count(BalaJ)
        if n > antes global.disparos += n - antes
        antes = n
    }
}
```

```sh
modengine check RogueShot --room mesa --seed 1 --no-save --steps 240 --test-script sonda.ms \
    --input '1:mouse=1150,330, 60-180:mb_left' \
    --assert '59:global.disparos == 0' --assert '180:global.disparos >= 3' --assert '*:exists(Jugador)'
```

Lo mismo con `run ... --screenshot-at 50,100,150` enseña la mira en (1150, 330), el arma
apuntando hacia ella y el tambor vaciándose.

**Carga (mantener y soltar)**: `--input "10-100:mb_left"` y `--assert '101:Jugador.carga >= 1'`.

**Ráfaga de clics**: `--input "10:mb_left,14:mb_left,18:mb_left"` (cada clic dura un paso).

**Panel con clic**: saca una captura para ver dónde está el botón (las coordenadas de la
captura son las GUI), acerca el ratón (muchos menús solo reaccionan si el ratón se *mueve*)
y haz clic:

```sh
modengine run RogueShot --no-save --input '190-200:mouse=640,700>810,570, 215:mb_left' \
    --screenshot-at 205,280 --screenshot clic.png --assert '280:Menu.pantalla == "creditos"'
```

**Serie de capturas** del menú, del nivel y de un momento concreto, sin ventanas:

```sh
modengine run MiJuego --seed 3 --no-save --input "60:enter, 100-400:right" \
    --screenshot-at 30,120,240,400 --screenshot capturas/serie_%04d.png
```

**Reproducibilidad**: dos ejecuciones con la misma `--seed` y la misma `--input` deben dar
capturas idénticas byte a byte (`cmp`) y el mismo `--watch`.

**Empezar en un estado concreto**: `--room R --exec 'global.nivel = 5'` o
`--exec-at '0:with Jugador { x = 800; vida = 1 }'`.

**Encontrar lo que va lento**: `modengine check MiJuego --steps 1200 --profile --input ...`
y mira las primeras filas.

**CI / sin pantalla**: `modengine check` (sin `--render`) no necesita servidor gráfico;
para capturas, `xvfb-run modengine run ... --screenshot-at ...`.
