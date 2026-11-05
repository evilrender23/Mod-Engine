# Editor visual (`modeditor`)

```bash
cargo build --release              # compila el motor, el editor y el ejecutable del juego
./target/release/modeditor examples/platformer
```

Sin argumentos abre una pantalla de inicio para abrir un proyecto (con lista de recientes) o
crear uno nuevo. El editor trabaja directamente sobre los ficheros del proyecto
(`game.toml`, `sprites/`, `rooms/`, `.ms`...), así que puedes alternar entre el editor y tu
editor de texto favorito.

## La ventana

| Zona | Qué hay |
|---|---|
| Arriba | Menús (Archivo, Crear, Juego, Editar, Workspace, Ver) y los botones ▶ Ejecutar y ✔ Comprobar |
| Izquierda | **Inspector**: miniatura y datos del asset seleccionado (tamaño, fotogramas, padre, eventos, hijos, variables con tipo, capas de la room…) y *Abrir editor* |
| Derecha | Navegador de assets: sprites (con miniatura), objetos (con su padre), rooms (★ = la inicial), sonidos, scripts. Clic = verlo en el inspector, **doble clic = abrirlo en el workspace**. **+** crea uno nuevo; clic derecho → Eliminar |
| Centro | **Workspaces**: pestañas *Workspace 1, 2…* y, en cada una, un lienzo con los editores como ventanas flotantes |
| Abajo | Salida: problemas del proyecto, mensajes de `print()` y errores del juego en marcha |

Los paneles laterales se ocultan desde **Ver**, donde también está el panel **Agente** (ver [Agente](#agente)).

## Workspaces

Funcionan como los de GameMaker:

![Workspace con cuatro editores encadenados](img/editor_workspace.png)

- Cada asset se abre como una **ventana** dentro del workspace: se arrastra por el título, se
  redimensiona desde el borde derecho, el inferior o la esquina, ▾ la pliega y 🗖 (o doble clic
  en el título) la maximiza dentro del workspace. ● en el título = cambios sin guardar.
- Lo que abres **desde** otra ventana (por ejemplo *Abrir objeto* en el editor de rooms, o
  *Abrir Actor* en un objeto) aparece a su derecha, **encadenado** con una línea.
- El fondo se arrastra para moverse; **Ctrl + rueda** hace zoom alrededor del cursor (25–200 %);
  la rueda sola desplaza. Clic derecho en el fondo → *Ver todas las ventanas* (Ctrl+0),
  *Zoom al 100 %* o *Cerrar todas*.
- **+** añade otro workspace; doble clic en su pestaña lo renombra; × lo cierra (cerrando sus
  ventanas). Si un asset ya está abierto en cualquier workspace, abrirlo de nuevo te lleva a él.
- Ctrl+Z / Ctrl+Y / Ctrl+W y Supr actúan sobre la ventana con foco (borde azul).
- La disposición (workspaces, ventanas, cadenas, zoom y paneles visibles) se guarda por proyecto
  en `~/.config/modengine/layouts/` y se recupera al abrirlo.

| Atajo | |
|---|---|
| Ctrl+S | guardar todo |
| F5 | guardar y ejecutar el juego (vuelve a pulsarlo para reiniciarlo) |
| F6 | comprobar: carga el proyecto con el motor real y ejecuta 300 pasos sin ventana |
| Ctrl+Z / Ctrl+Y | deshacer / rehacer (rooms y sprites; los campos de texto tienen su propio deshacer) |
| Ctrl+W | cerrar la ventana con foco |
| Ctrl+0 | ver todas las ventanas del workspace |

Al guardar, el editor vuelve a cargar el proyecto con el motor y avisa en la salida si algo no
compila. Lo que se elimina no se borra: se mueve a la carpeta `.trash/` del proyecto.

## Editor de rooms

![Editor de rooms](img/editor_room.png)

- **Capas** (izquierda): instancias, tiles y fondo. El ojo las oculta, ▲▼ las ordena en la lista;
  el orden de dibujo lo marca la profundidad. Debajo, las propiedades de la capa elegida y las de
  la room: tamaño (automático o fijo), color de fondo, pasos por segundo, persistencia, a quién
  sigue la cámara y el código de creación.
- **Paleta** (derecha): los objetos con su sprite, o los tiles del tileset de la capa.
- **Herramientas**:

| | |
|---|---|
| ✏ Pintar | coloca el objeto/tile elegido. Con *ajustar a rejilla* va al mapa de celdas; sin él crea una instancia libre donde haces clic |
| ⌫ Borrar | quita celdas e instancias libres (el clic derecho borra con cualquier herramienta) |
| ⬚ Mover | selecciona una instancia libre, arrástrala, edita su objeto, posición y variables propias; Supr la elimina |
| ▭ Rectángulo | rellena un rectángulo de celdas |
| 🪣 Rellenar | rellena la zona conectada de celdas iguales |
| 💧 Cuentagotas | toma el objeto o tile que hay bajo el cursor |

Rueda = zoom alrededor del cursor; botón central o Espacio + arrastrar = desplazar. El rectángulo
azul es la cámara al empezar la room.

Las instancias en rejilla se guardan como el mapa ASCII de siempre (el editor elige los
caracteres de la leyenda y reutiliza los que ya había); las libres, en la lista `instances`.

## Editor de objetos

![Editor de objetos](img/editor_object.png)

- Sprite, máscara y padre en desplegables; visible, sólido y persistente; profundidad.
- **Eventos**: **+ Evento** ofrece todos (colisión con cada objeto, cada tecla, alarmas, ratón,
  eventos de usuario…). Debajo, en gris, los eventos que el objeto **hereda** de sus padres:
  al seleccionarlos se ve su código y *Sobrescribir aquí* crea uno propio que empieza con
  `inherited()`.
- **Métodos** y **Variables**: cada variable tiene nombre, tipo opcional (`num`, `int`, `bool`,
  `str`, `color`, `list`, o un tipo de asset) y valor inicial con el control de su tipo:
  deslizador si el número tiene límites, casilla, selector de color, desplegable... Si el valor
  es una expresión (`room_width / 2`) se edita como texto. Debajo aparecen en gris las
  variables con tipo **heredadas** del padre, con *Sobrescribir* para darles otro valor aquí.
- En el editor de rooms, al seleccionar una instancia libre, **Variables del objeto** muestra
  todas las variables con tipo de su objeto con su valor; cambiar una lo guarda solo para esa
  instancia, y ↺ vuelve al valor del objeto.
- El código se edita con resaltado de sintaxis; cualquier error de compilación aparece arriba
  con su línea.

El editor modifica solo el bloque `object ... { }` dentro de su fichero `.ms`: el resto del
fichero (otras funciones, comentarios) queda igual. El cuerpo de cada evento se guarda tal cual
lo escribes. Si un objeto usa algo que el editor visual no entiende, ofrece abrir el fichero
como texto.

## Editor de sprites

![Editor de sprites](img/editor_sprite.png)

Para sprites de pixel art: lápiz, borrador, relleno, cuentagotas y herramienta de origen;
rejilla, papel cebolla (el fotograma anterior en transparencia), voltear, fotogramas (nuevo,
duplicar, eliminar, reordenar), vista previa animada con sus fps, colores de la paleta común
más colores propios del sprite, cambio de tamaño, origen y máscara de colisión manual o
automática.

Para sprites PNG: tira o rejilla de fotogramas, fps, origen y máscara.

## Sonidos, scripts y ajustes

- **Sonidos**: todos los parámetros del sintetizador con deslizadores, ▶ Escuchar (usa
  `pw-play`, `paplay` o `aplay`) y unos cuantos puntos de partida (salto, moneda, golpe, láser).
- **Scripts**: editor de código con resaltado (los colores `#rrggbb` se muestran en su color) y
  comprobación de sintaxis en vivo.
- **Ajustes del juego**: nombre, room inicial, orden de las rooms, resolución, escala, pasos por
  segundo y la paleta compartida de pixel art.

## Agente

El panel **Agente** (a la derecha; **✦ Agente** en la barra de arriba o **Ver → Agente**) conversa
con un agente de programación que trabaja en la carpeta del proyecto y escribe y edita sus objetos,
rooms, sprites, sonidos y scripts: **Claude Code** (`claude`), **Antigravity** (`agy`) o **Codex**
(`codex`), elegido en el desplegable de arriba. Hay que tener instalado su programa y haber
iniciado sesión en él una vez desde un terminal; si falta, el panel dice cómo instalarlo y tiene
*Comprobar de nuevo*.

- **Mensajes:** Intro envía (Mayús+Intro, nueva línea). La respuesta llega a medida que se escribe,
  con una línea por cada cosa que hace el agente (leer, editar, ejecutar…) y avisos de lo que
  falla o bloquean los permisos. **＋ Nueva** empieza otra conversación, **■ Detener** para el
  agente (y los comandos que haya lanzado) y **⧉** copia la orden para seguir la conversación en
  un terminal (`claude --resume …`, `agy --conversation …`, `codex resume …`).
- **Modelo** (bajo el mensaje): *Por defecto* usa el que tenga configurado el propio programa.
  Claude Code ofrece Fable, Opus, Sonnet y Haiku (siempre la última versión de cada uno); Codex y
  Antigravity enseñan la lista que dan `codex debug models` y `agy models`. *Otro modelo…* acepta
  cualquier nombre que el programa admita tras `--model`. La conversación indica qué modelo
  contesta cuando lo dice el programa.
- **Permisos:** *Solo planificar* (lee y propone), *Editar archivos* o *Editar y ejecutar
  comandos*, que es el modo por defecto. Ninguno se salta las aprobaciones del programa.
- **Contexto:** el agente sabe que está en un proyecto de Mod Engine, cómo se organiza y dónde
  están `docs/lenguaje.md`, `docs/funciones.md` y `docs/proyecto.md` (con permiso para leerlos
  aunque estén fuera del proyecto). Las busca en `$MODENGINE_DOCS`, junto al ejecutable, en la
  instalación o en el código del que se compiló el editor.
- **Lo que ve y lo que cambia:** al enviar, el editor **guarda todo** para que el agente trabaje
  sobre lo que ves. Cada vez que el agente dice que ha escrito un fichero (o ha ejecutado un
  comando), y al terminar, el editor recarga del disco los scripts, `game.toml`, las listas de
  assets, los sprites y los editores abiertos de rooms, sprites, sonidos y ajustes cuyo fichero
  cambió. Al terminar, además, comprueba el proyecto y deja los errores del motor en **Salida**,
  junto con la lista de ficheros que cambió el agente.
- **Cambios sin guardar:** no se pisan. Si el agente cambia un fichero que tiene cambios sin
  guardar en el editor (cosa rara, porque se guarda al enviar), se conservan los tuyos y Salida lo
  avisa; guardar entonces sobrescribe la versión del agente.
- Agente, modelo de cada agente, permisos y la última lista de modelos de cada programa se
  recuerdan en `~/.config/modengine/agent.toml`. Cerrar el proyecto o el editor detiene el agente.

Cómo se lanza cada programa (en la carpeta del proyecto, sin ventana):

| Agente | Orden |
|---|---|
| Claude Code | `claude --print --output-format stream-json --verbose --include-partial-messages --permission-mode plan\|acceptEdits\|auto --append-system-prompt-file <contexto> [--model M] [--resume ID] --allowedTools "Read(//<docs>/**)"`, mensaje por stdin |
| Antigravity | `agy --output-format stream-json --mode plan\|accept-edits\|auto --settings <permisos JSON> [--model M] [--conversation ID] --print=<contexto + mensaje>` |
| Codex | `codex exec --json --skip-git-repo-check --sandbox read-only\|workspace-write [--model M] [resume ID] -`, contexto + mensaje por stdin |

## Tema

**Ver → Tema claro / Tema oscuro.** El claro (por defecto) reproduce el tema claro de Hermes
Agent Desktop: fondos azulados muy suaves (`#f8f9fe`, barra lateral `#f3f7ff`), superficie de
edición blanca, acento `#0053fd`, bordes con un tinte del acento y código con los colores de
GitHub Light. La elección se guarda en `~/.config/modengine/theme.txt`. Los colores están en
`crates/modengine-editor/src/theme.rs`.

## Formato de los ficheros guardados

El editor escribe TOML ordenado y legible (mapas y pixel art como bloques `'''`). Los
comentarios que hubiera en un `.toml` editado desde el editor se pierden al guardar; los de
los `.ms` se conservan.

## Tests

```bash
cargo test -p modengine-editor
```

Además de comprobar que todos los objetos y rooms del ejemplo se leen y se vuelven a escribir sin
cambios, hay tests que manejan la interfaz con ratón simulado (`egui_kittest`): pintar y borrar
en una room, deshacer y guardar; pintar un píxel; añadir un evento desde el menú.
Los del panel Agente usan un `claude` falso (un script de shell) que guarda sus argumentos y su
stdin y escribe JSON como el de verdad; no hace falta red ni los programas reales. Para probar
uno real: `MODENGINE_AGENT_SMOKE=claude cargo test -p modengine-editor -- --ignored smoke`
(o `codex`, `agy`).

Para capturas automáticas: `modeditor <proyecto> --open room:level1 --screenshot captura.png`
(`--open` acepta `room:`, `object:`, `sprite:`, `sound:`, `script:` y `settings`; `--agent` muestra el panel Agente).
