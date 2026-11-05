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

Orden de búsqueda de un nombre al leerlo: variable local → variable de `self` → función global
→ asset o constante → función integrada. Al asignar: local si se declaró con `var`; si no,
variable de `self` (o global si el código corre en el ámbito global).

## modengine

`Engine` lo contiene todo: assets, definiciones de objetos y rooms, instancias vivas, cámara,
input y la lista de órdenes de dibujo. No depende de ninguna librería gráfica.

- **Objetos** (`object.rs`): al cargar se resuelve la cadena de padres y se calcula la tabla de
  eventos efectiva de cada objeto (los del hijo sustituyen a los del padre). `inherited()` usa
  una pila de eventos en curso para encontrar la versión del padre.
- **Instancias** (`instance.rs`): las variables integradas son campos de Rust; el resto va a un
  `HashMap<Sym, Value>`. Las instancias destruidas se marcan y se eliminan al final del paso.
- **Rooms**: se construyen desde el TOML (mapas ASCII incluidos). Al salir de una room
  persistente se guardan sus instancias y capas.

### Orden de un paso (`Engine::tick`)

1. Se guarda `xprev`/`yprev` de cada instancia.
2. `step_begin`
3. Alarmas: cada contador > 0 baja 1; al llegar a 0 se pone a -1 y se dispara `alarm(n)`.
4. Eventos de teclado y ratón.
5. `step`
6. Movimiento automático: fricción, gravedad, `x += hspeed`, `y += vspeed`, animación
   (y `animation_end`).
7. Colisiones: para cada evento `collision(Obj)` y cada instancia que se solape. Si la otra es
   `solid`, la instancia vuelve a `xprev`/`yprev` antes del evento.
8. `outside_room` y `boundary`.
9. `step_end`
10. Desplazamiento de fondos, cámara, limpieza de instancias destruidas.
11. Si se pidió, cambio de room: `room_end`, se eliminan las instancias no persistentes, se
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
- Las pulsaciones se acumulan entre pasos para que ninguna se pierda ni se cuente dos veces
  cuando los fps y los pasos por segundo no coinciden.
- `check` ejecuta el mismo motor sin ventana: los dibujos se descartan y el texto se mide con
  una aproximación.

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
- Exportar a web (macroquad compila a WebAssembly).
