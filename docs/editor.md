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

Los tres paneles laterales se ocultan desde **Ver**.

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

Para capturas automáticas: `modeditor <proyecto> --open room:level1 --screenshot captura.png`
(`--open` acepta `room:`, `object:`, `sprite:`, `sound:`, `script:` y `settings`).
