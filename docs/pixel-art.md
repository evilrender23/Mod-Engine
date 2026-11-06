# Dibujar pixel art mediante código

Guía para crear y revisar pixel art escribiéndolo como datos: sprites, iconos, tiles y
animaciones sobre una cuadrícula de píxeles, o corregir la legibilidad y coherencia de assets
existentes. La sigue el panel **Agente** del editor (ver [editor.md](editor.md#pixel-art-con-el-agente))
cuando le pides dibujar o arreglar sprites; sirve igual para hacerlo a mano. Al final,
[En Mod Engine](#en-mod-engine) cuenta cómo se aplica cada paso con los ficheros del motor y
cómo se verifica con `modengine pixel-check`.

## 1. Fijar las restricciones antes de dibujar

- Extrae del encargo el objeto, la acción o pose, la vista, la resolución nativa, el fondo y el formato de entrega. Respeta las dimensiones exactas que indique el usuario.
- Si trabajas en un juego existente, toma un sprite de la misma familia como referencia. Comprueba cuánto espacio ocupa dentro de su celda, dónde apoya los pies, qué grosor tiene el contorno y desde dónde recibe la luz.
- Si solo tienes una descripción, elige una resolución que permita reconocer el rasgo principal. Para un personaje sencillo, 32 × 32 px es un punto de partida. Ajusta esa decisión al encargo.
- Identifica el rasgo que debe sobrevivir a la reducción. Para una llave puede ser la muesca del diente. Para una herrera pequeña puede ser el perfil del martillo sobre el hombro.

## 2. Escribir el sprite sobre su cuadrícula nativa

- Trabaja con coordenadas enteras. Fija el origen en la esquina superior izquierda, salvo que el proyecto use otra convención.
- Para sprites pequeños dibujados a mano, usa una matriz de caracteres y una paleta explícita. Asigna un carácter a cada color y reserva `.` para transparencia. Cada fila debe tener exactamente el ancho declarado.
- Para formas grandes o repetidas, usa primitivas enteras si facilitan la edición. Conserva el control individual de los píxeles donde cambie la lectura de la forma, como ojos, dedos, puntas y esquinas.
- Separa los datos del dibujo de la función que los renderiza. Cambiar el color de una chaqueta o corregir una fila debe ser una edición localizada.
- Dibuja directamente a la resolución de destino. Amplía la vista de trabajo con vecino más cercano y escala entera.

## 3. Construir la imagen por pasadas

1. **Silueta.** Rellena la figura con un único color. Comprueba la orientación, la pose y el rasgo distintivo. Si una herramienta se pierde contra el torso, desplázala un píxel o abre un hueco entre ambas formas.
2. **Masas de color.** Divide la silueta en pocas regiones conectadas. En un personaje, empieza por pelo, cara, ropa y accesorio. Resuelve primero dónde termina cada forma.
3. **Claridad y sombra.** Elige una dirección de luz. Coloca la sombra donde una forma tape la luz o gire respecto a ella. Por ejemplo, una franja corta bajo un flequillo puede bastar para separarlo de la frente. Evita sombrear todos los bordes por igual salvo que busques ese efecto.
4. **Contornos.** En una diagonal recta, mantén un ritmo regular de escalones. En una curva, cambia su longitud de manera gradual. Conserva una irregularidad cuando describa algo concreto, como una muesca en una hoja.
5. **Detalle.** Añade cada píxel aislado con una función visible: una pupila, una hebilla, el brillo de un filo. Usa tramado o suavizado manual cuando ayuden a describir un material o borde a tamaño de juego.
6. **Revisión a escala final.** Mira el sprite sobre el fondo previsto. Si se pierde un rasgo, corrige primero su posición, tamaño o contraste. Añade textura solo después de resolver esa lectura.

## 4. Aplicar las reglas específicas del asset

- **Personajes de una misma familia.** Conserva las proporciones solicitadas, la perspectiva, la escala y la línea de apoyo. Diferencia personajes mediante rasgos que ocupen varios píxeles, como la caída de una capucha o la anchura de unas mangas.
- **Animación.** Planifica las poses clave antes de generar intermedios. Mantén fija la celda y define un anclaje. Durante el apoyo de una pisada, revisa la posición del pie teniendo en cuenta el desplazamiento del personaje. Reproduce el bucle y comprueba también la transición del último fotograma al primero.
- **Tiles repetibles.** Monta una prueba de 3 × 3 copias. Revisa las uniones y busca manchas que formen bandas o dibujos involuntarios al repetirse.
- **Iconos.** Reserva el mayor contraste para el rasgo que distingue el objeto. Compara los iconos de una misma familia lado a lado y al tamaño real de la interfaz.

## 5. Verificar y entregar

- Comprueba por código las dimensiones, la longitud de las filas, la correspondencia entre símbolos y paleta, y los límites de las coordenadas.
- Si la paleta es cerrada, compara los colores visibles del archivo renderizado con los permitidos. Si se exige transparencia binaria, comprueba que el canal alfa solo contenga 0 y 255.
- Renderiza e inspecciona el resultado a tamaño nativo y ampliado. Con fondo transparente, pruébalo sobre claro y oscuro para detectar huecos o halos.
- En una hoja de sprites, verifica el número y orden de los fotogramas, el tamaño de celda y la posición del anclaje.
- Entrega el formato pedido. Si el usuario pide código, incluye los datos completos del dibujo y lo necesario para renderizarlo. Si pide un PNG, expórtalo a resolución nativa. Identifica por separado cualquier previsualización ampliada.
- Distingue las comprobaciones ejecutadas de las pendientes. La validación de dimensiones y colores no sustituye la inspección visual.

## Ejemplo de decisión

Encargo: personaje frontal con cabeza y torso cuadrados del mismo tamaño y dos piernas cortas.

Conserva ambos cuadrados. Si parecen un solo bloque, prueba primero a separar la cara de la camiseta mediante su claridad o a marcar la barbilla con una sombra corta. Deja un hueco visible entre las piernas. Compara las variantes a tamaño nativo y elige la que conserve mejor las proporciones y la pose solicitadas.

## En Mod Engine

Cómo se traduce cada paso a los ficheros del motor (formato completo en
[proyecto.md](proyecto.md#sprites-sprites)).

**La matriz de caracteres es el sprite.** Un sprite de texto es `sprites/<nombre>.toml` con
`pixels`: cada carácter es un píxel y cada bloque `'''` es un frame. Datos y dibujo ya están
separados: el motor lo renderiza al cargar, y cambiar un color es cambiar una entrada de
paleta.

```toml
# sprites/llave.toml: 16 × 16, colores de la paleta del proyecto (game.toml)
origin = "center"
fps = 0
pixels = ['''
................
.....kkkk.......
....kyyyyk......
...kyykkyyk.....
...kyk..kyk.....
...kyykkyyk.....
....kyyyyk......
.....kyyk.......
.....kyyk.......
.....kyyk.......
.....kyykkk.....
.....kyyyyyk....
.....kyykkk.....
.....kyyyk......
.....kkkkk......
................
''']
```

- **Paleta.** Usa primero los caracteres de `[palette]` de `game.toml` (los proyectos nuevos
  traen Pear36: `0`-`9` y `A`-`Z`). Una `palette = { … }` propia del sprite es solo para colores
  que la del proyecto no tiene; `pixel-check` avisa si un color local repite uno del proyecto.
  `.` es transparente; no uses espacios (los del principio y el final de una fila se pierden).
- **Ancho exacto.** Todas las filas de todos los frames deben medir lo mismo. El motor rellena
  en silencio las filas cortas con transparente, así que un error de un carácter desplaza el
  dibujo sin fallar al cargar: compruébalo siempre con `pixel-check`.
- **Resolución nativa.** Dibuja al tamaño de juego, sin escalar el arte a mano. Para verlo más
  grande en el juego, escala la instancia (`scale_x`, `scale_y`), con `[display] scale` o, en capas de
  tiles, con `scale = 3`; nunca dupliques píxeles en el TOML.
- **Anclaje.** `origin` es el ancla (`"bottom"` para lo que apoya en el suelo: los pies en la
  última fila; `"center"` para lo que gira o vuela; o `[x, y]`). Todos los frames comparten celda
  y origen. `mask` es la caja de colisión; `fps` la velocidad (`0`: los frames son variantes).
- **Animación.** Un bloque por frame, en orden; el último enlaza con el primero.
- **Tiles.** Las piezas repetibles van en un tile set (`tilesets/<nombre>.toml` con `sprite` y
  `tile_width`/`tile_height`, ver [proyecto.md](proyecto.md#tile-sets-tilesets)); dibuja cada tile como frame de un
  sprite de texto y compruébalo con `--tile`.
- **Sprites grandes.** Con `pixels_rle` (tramos `C*N`) o `modengine sprite-bake` pasan a texto
  compacto o PNG; `pixel-check` revisa igual los PNG (colores, alfa, frames, anclaje).

### Verificar con `modengine pixel-check`

```sh
modengine pixel-check <proyecto> llave --preview auto        # un sprite (nombre, sprites/x.toml o un .png)
modengine pixel-check <proyecto> --all                       # todos
modengine pixel-check <proyecto> suelo --tile --preview /tmp/px   # tiles: uniones y prueba 3×3
modengine pixel-check <proyecto> llave --size 16x16 --palette pear36 --json
```

Cada comprobación sale como `ok`, `info`, `aviso`, `error` u `omitida`, con el lugar
(`frame 0, fila 3, col 5`, todo contado desde 0):

| Comprobación | Qué mira |
|---|---|
| nombre (`name`) | el nombre no es el de una constante o función integrada: un sprite `key` taparía `key.escape` en todos los scripts y el proyecto dejaría de cargar |
| ancho de filas (`row_width`) | cada fila mide exactamente el ancho del sprite (error con frame, fila y longitud) |
| tamaño de frames (`frame_size`), tamaño pedido (`size`) | todos los frames igual de altos; `--size WxH` exige ese tamaño |
| símbolos (`symbols`) | caracteres que no están en la paleta, con su posición |
| entradas de paleta (`palette_entries`) | colores mal escritos, colores locales sin usar o que repiten / tapan los del proyecto |
| paleta cerrada (`closed_palette`) | colores visibles del sprite renderizado (también PNG) dentro de los permitidos: por defecto la paleta del proyecto (aviso); con `--palette` (colores, `pear36`, un archivo .hex/.gpl/.pal/.json o una paleta guardada del editor) es error |
| alfa binaria (`alpha`) | los PNG solo con alfa 0 y 255 (error con `--binary-alpha`) |
| origen y máscara (`origin`, `mask`) | dentro de la celda |
| frames (`frames`) | número de frames y tamaño de celda frente a la imagen (`frames`, `frame_size`) |
| frames vacíos (`empty_frames`) | frames sin ningún píxel visible |
| anclaje (`anchor`) | con origen abajo, la fila más baja con píxeles de cada frame (aviso si cambia entre frames o no toca el origen); si no, la caja del contenido por frame |
| píxeles sueltos (`isolated_pixels`) | motas sin vecinos y píxeles sin vecino del mismo color (solo informa: cada uno debe tener función) |
| uniones del tile (`tile_seams`, `--tile`) | bordes izquierdo/derecho y arriba/abajo que no casan y huecos en el borde |

`--preview DIR` (o `auto`: `<temp>/modengine-pixel-check/<proyecto>`) escribe PNG para mirarlos,
con vecino más cercano y escala entera (`--scale N`; por defecto, frames de 128 px o más):

- `<nombre>_x<escala>.png`: todos los frames en fila, ampliados, sobre un damero.
- `<nombre>_fondos.png`: tamaño nativo y ampliado, sobre fondo claro (arriba) y oscuro (abajo),
  para ver huecos y halos.
- `<nombre>_bucle.png` (animaciones): el último frame junto al primero y, para cada frame, piel
  de cebolla: el anterior en rojo, el actual en azul y el origen en verde.
- `<nombre>_3x3.png` (`--tile`): cada frame repetido 3 × 3.

Las vistas previas nunca se escriben dentro del proyecto (un PNG en `sprites/` sería un sprite
nuevo). El texto termina con lo **pendiente**: lo que solo se juzga mirando (lectura del rasgo
principal, halos, el bucle, bandas al repetir). Códigos de salida: 0 bien o solo avisos, 1
errores o un sprite que no existe, 2 argumentos mal. `--json` da lo mismo para scripts y agentes.
