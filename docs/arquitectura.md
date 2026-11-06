# Arquitectura

```text
modscript            lenguaje: lexer → parser → AST → intérprete
   ▲  trait Host     (el intérprete pide al motor instancias, assets y funciones)
modengine            Engine: proyecto, objetos, rooms, instancias, bucle de juego, natives
   ▲  DrawCmd / AudioCmd / Input
modengine-cli        ventana, dibujo, audio e input con macroquad; modo `check` sin ventana
```

## modscript

- `lexer.rs` convierte el texto en tokens (`#rrggbb` es un número, `and/or/not/mod/div` son operadores).
- `parser.rs` es descendente recursivo con precedencia de operadores. Produce `FileAst`:
  objetos, funciones globales y código suelto.
- `interp.rs` recorre el árbol. Todo lo que tiene que ver con el juego lo delega en el trait
  `Host`: leer/escribir variables de instancia, resolver nombres de assets, llamar funciones
  integradas y listar instancias para `with`.
- Los identificadores se internan (`Sym`), así que buscar una variable compara enteros.
- Cierres (`freevars.rs` + `make_closure` en `interp.rs`): al crear una función se miran los
  nombres libres de su cuerpo (calculados una vez por `FuncDef`); los que son locales del
  código que la crea pasan a una celda compartida (`Rc<RefCell<Value>>`) que guarda la
  `Closure`. Las locales que nadie captura siguen siendo valores normales en la pila.

Orden de búsqueda de un nombre al leerlo: variable local → variable de `self` → función global
→ asset o constante → función integrada. Al asignar: local si se declaró con `var`; si no,
variable de `self` (o global si el código corre en el ámbito global).

## modengine

`Engine` lo contiene todo: assets, definiciones de objetos y rooms, instancias vivas, cámara,
input y la lista de órdenes de dibujo. No depende de ninguna librería gráfica.

- **Objetos** (`object.rs`): al cargar se resuelve la cadena de padres y se calcula la tabla de
  eventos efectiva de cada objeto (los del hijo sustituyen a los del padre). `inherited()` usa
  una pila de eventos en curso para encontrar la versión del padre. En métodos, `inherited()` y
  `super.m()` usan `methods.rs`: cada cuerpo de método o evento está asociado al objeto que lo
  declara, y la búsqueda sube desde el padre de ese objeto (`Host::super_method`).
- **Análisis estático** (`lint.rs`): `modengine check` recorre todo el código dos veces (reúne
  qué nombres se asignan en algún sitio; luego avisa de lo que no existe). No ejecuta nada.
- **Instancias** (`instance.rs`): las variables integradas son campos de Rust; el resto va a un
  `HashMap<Sym, Value>`. Las instancias destruidas se marcan y se eliminan al final del paso.
- **Rooms**: se construyen desde el TOML (mapas ASCII incluidos). Al salir de una room
  persistente se guardan sus instancias y capas.

### Orden de un paso (`Engine::tick`)

1. Se guarda `xprev`/`yprev` de cada instancia.
2. Texto tecleado (`keyboard_string`) y **red** (`net.rs`): se aceptan conexiones, se lee y se
   escribe en todos los sockets sin bloquear, y por cada cosa ocurrida (conexión, paquete,
   desconexión…) se ejecuta el evento `network` de todas las instancias con `async_load` relleno.
3. `step_begin`
4. Alarmas: cada contador > 0 baja 1; al llegar a 0 se pone a -1 y se dispara `alarm(n)`.
5. Eventos de teclado y ratón.
6. `step`
7. Movimiento automático: fricción, gravedad, `x += hspeed`, `y += vspeed`, animación
   (y `animation_end`).
8. Colisiones: para cada evento `collision(Obj)` y cada instancia que se solape. Si la otra es
   `solid`, la instancia vuelve a `xprev`/`yprev` antes del evento.
9. `outside_room` y `boundary`.
10. `step_end`
11. Desplazamiento de fondos, cámara, limpieza de instancias destruidas.
12. Si se pidió, cambio de room: `room_end`, se eliminan las instancias no persistentes, se
    crean las de la nueva room (propiedades → variables del mapa → `create`), código de la room,
    `room_start`.

Las instancias creadas durante un paso reciben eventos a partir del siguiente.

### Dibujo (`Engine::render`)

1. Cámara y color de fondo de la room.
2. `draw_begin` de todas las instancias visibles (de más profunda a menos).
3. Capas de fondo/tiles e instancias mezcladas por `depth`; cada instancia ejecuta `draw`, o se
   dibuja su sprite si no tiene ese evento.
4. `draw_end`, cajas de colisión si F3 está activo.
5. `draw_gui` en coordenadas de pantalla.

El resultado es una lista de `DrawCmd` (sprite, rectángulo, texto...). El backend la ejecuta.

## modengine-cli

- El juego se dibuja en una textura del tamaño de la resolución del juego y luego se escala a
  la ventana (por números enteros si `pixel_perfect`), con bandas en los bordes.
- La lógica corre a paso fijo (`step_rate`); el dibujo, una vez por frame si hubo algún paso.
- Sprites (`sprite_draw.rs`): al arrancar, las hojas de los sprites de hasta 1024 px se
  empaquetan en unas pocas texturas atlas (4096², 2048² en Android; las de filtrado lineal
  aparte), con 2 px de borde repetido para que el filtrado no lea al vecino. Así miles de
  instancias con sprites distintos salen en una sola llamada de dibujo **sin reordenar nada**
  (el orden por `depth` y de creación se respeta tal cual). Con un shader activo se usa la
  textura propia del sprite (las coordenadas `v_vTexcoord` siguen siendo las de su hoja).
  `MODENGINE_NO_ATLAS=1` desactiva el atlas (diagnóstico). Medido con
  `cargo run --release -p modengine-cli --example draw_bench` (sin argumentos crea 5000 balas
  de 4 sprites intercalados; también acepta `<proyecto> [frames]`): de 5000 cambios de textura
  por frame a 1, y de ~31–52 ms a ~4 ms por frame (RTX 4090, sin vsync, un paso lógico por
  frame; ~3,5 ms de ellos son la lógica y `render()` del motor).
- RotSprite (`modengine::rotsprite`): los sprites con `rotation = "rotsprite"` girados a un
  ángulo que no es múltiplo de 90° se dibujan desde una imagen calculada en CPU (Scale2x ×3,
  giro, reducción por moda) y guardada como textura por (sprite, frame, paso de ángulo), con
  un presupuesto de ~6 ms por frame para ángulos nuevos y ~96 MB de caché LRU.
- Las pulsaciones se acumulan entre pasos para que ninguna se pierda ni se cuente dos veces
  cuando los fps y los pasos por segundo no coinciden.
- `check` ejecuta el mismo motor sin ventana: los dibujos se descartan y el texto se mide con
  una aproximación.
- `server` (o `run --headless`, o `--headless` en un juego exportado) ejecuta solo la lógica, en
  tiempo real y sin dibujar: servidores dedicados de multijugador (`headless.rs`).
- Las opciones de prueba de la terminal (docs/pruebas.md) viven en `modengine::testing`
  (semilla, `--exec-at`, `--assert`, objetos de `--test-script` creados al entrar en cada room,
  perfil por evento medido en `exec_event`) y en `modengine-cli/src/test_run.rs` (opciones,
  capturas por paso lógico leyendo la imagen final del lienzo, ventana retirada del escritorio).
  Los tres bucles (`check`, `headless.rs`, la ventana) llaman a `test_tick`/`test_render` en vez
  de `tick`/`render`. Con `--screenshot-at` o `run --steps` la ventana va a paso fijo: un paso
  lógico por frame y sin vsync, independiente del tiempo real.
- La red usa `std::net` sin hilos ni async: sockets no bloqueantes revisados una vez por paso.
  Solo `network_connect` usa un hilo auxiliar (resolver el nombre y conectar), cuyo resultado
  llega como evento.

## Juegos exportados

- `modengine::pack` escribe y lee `game.dat`: el proyecto entero en un fichero (índice + datos +
  suma de comprobación), pegado al final del runner o en los assets del APK.
- `modengine::vfs` hace que los cargadores lean igual de una carpeta que de un pack montado en una
  ruta (la del propio fichero). Con una carpeta no se monta nada: el comportamiento no cambia.
- `modengine-cli` es también una biblioteca: `runner.rs` (busca el pack y abre la ventana),
  `touch.rs` (mando táctil), `export/` (`modengine export`). El binario `modengine-runner` es lo que
  se copia en los juegos exportados; en Android, la biblioteca se compila como `cdylib` y exporta
  `quad_main`. Ver [exportar.md](exportar.md).

## Añadir una función integrada

Las funciones están en `crates/modengine/src/natives/` (`core.rs`, `game.rs`, `gfx.rs`):

```rust
r.f("double", |_engine, _cx, a| {
    let v = A(a).num(0)?;          // argumento 0 como número, con error claro si no lo es
    Ok(Value::Num(v * 2.0))
});
```

`cx.self_` es la instancia que llama; `me(engine, cx)?` da su índice en `engine.insts`.

## Ideas para seguir

- Editor visual (rooms, sprites, objetos) sobre el mismo formato de proyecto.
- Máscaras de colisión por píxel y cajas que rotan.
- Rejilla espacial para colisiones con miles de instancias.
- Mandos (gamepad), shaders, sistemas de partículas, caminos (paths) y timelines.
- Exportar a web (macroquad compila a WebAssembly) y a macOS.
