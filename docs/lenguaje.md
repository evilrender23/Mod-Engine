# ModScript

ModScript es el lenguaje de Mod Engine. Es dinámico (las variables no tienen tipo fijo), con
sintaxis parecida a C/JavaScript, y está pensado para escribir objetos de juego: bloques de
eventos, variables de instancia, `self`/`other` y `with`.

Los ficheros `.ms` pueden estar en cualquier carpeta del proyecto; el motor los carga todos en
orden alfabético de ruta.

## Lo que puede haber en un fichero

```js
// 1. Objetos
object Player : Actor { ... }

// 2. Funciones globales: se pueden llamar desde cualquier sitio
function clamp_speed(v) { return clamp(v, -4, 4) }

// 3. Código suelto: se ejecuta una vez al arrancar el juego, en el ámbito global
global.score = 0
```

## Valores

| Tipo | Ejemplos |
|---|---|
| número | `3`, `-0.5`, `1_000`, `0xff`, `#ff8800` (color RGB) |
| texto | `"hola"`, `'hola'`, escapes `\n \t \" \\` |
| booleano | `true`, `false` |
| nulo | `null` (o `none`) |
| array | `[1, 2, 3]` |
| struct | `{ name: "slime", hp: 3 }` |
| función | `function(a, b) { return a + b }` |
| instancia | lo que devuelve `spawn()`, `id`, `self`, `other` |
| asset | nombres de objetos, sprites, sonidos, rooms y fuentes: `Player`, `coin`, `snd_jump`, `level1` |

Cualquier valor es "verdadero" salvo `false`, `null`, `0` y `""`.

## Variables

```js
hp = 3              // variable de la instancia que ejecuta el código (self)
var t = 0           // variable local: solo existe mientras dura el evento o la función
global.lives = 3    // global: visible desde todo el juego
other.hp -= 1       // variable de otra instancia
```

Leer una variable que nunca se ha asignado es un error con línea y fichero, para que los
errores tipográficos no pasen desapercibidos. `x ??= 0` asigna solo si no existe o es `null`.

Las variables integradas de cada instancia (posición, velocidad, sprite...) están en
[funciones.md](funciones.md#variables-de-instancia).

## Operadores

```text
+ - * / %        aritmética (también `mod`, y `div` = división entera)
== != < <= > >=  comparación
&& || !          lógica (también `and`, `or`, `not`)
& | ^ ~ << >>    bits
a ?? b           b si a es null
c ? a : b        condicional
= += -= *= /= %= ??=   asignación
x++  x--  ++x  --x
```

`"texto" + 5` concatena. `"ab" * 3` repite el texto.

## Control de flujo

```js
if hp <= 0 { destroy() } else if hp < 3 { tint = #ff0000 }
if dead exit                       // las llaves son opcionales con una sola sentencia

while vsp > 0 { vsp -= 1 }
do { n++ } until n >= 10
for (var i = 0; i < 10; i++) { ... }
for (e in enemigos) { e.hp -= 1 }          // cada elemento de un array
for (i, e in enemigos) { ... }             // índice y elemento
for (k, v in stats) { ... }                // clave y valor de un struct
repeat 5 { spawn(Spark, x, y) }

switch state {
    case "idle": sprite = hero_idle; break
    case "run":  sprite = hero_run;  break
    default:     sprite = hero_jump
}
```

`break` y `continue` funcionan en todos los bucles; `exit` termina el evento o función actual;
`return valor` devuelve un valor (el valor debe empezar en la misma línea que `return`).
El `;` al final de las sentencias es opcional.

### `for (x in ...)`

| Se recorre | `for (x in c)` | `for (a, b in c)` |
|---|---|---|
| array | `x` = cada elemento | `a` = índice, `b` = elemento |
| struct | `x` = cada clave (texto) | `a` = clave, `b` = valor |
| texto | `x` = cada carácter | `a` = índice, `b` = carácter |
| objeto, instancia o `all` | `x` = cada instancia viva | `a` = índice, `b` = instancia |

- Se recorre una **copia** tomada al empezar: añadir o quitar elementos (o claves) dentro del
  bucle no cambia las vueltas que quedan. Los arrays y structs que hay *dentro* sí son los
  mismos, así que `e.hp -= 1` modifica el original; asignar a la variable (`e = 5`) no.
- En objetos, las instancias destruidas durante el bucle se saltan.
- Recorrer `null` (o un número) es un error: «for-in needs an array, struct, string or object».
- La variable del bucle es una local más (con o sin `var` delante); al acabar conserva el
  último valor. Cada vuelta es una variable nueva para las funciones anónimas creadas dentro
  (ver *Cierres*).
- `in` solo es palabra clave dentro de la cabecera del `for`: puede seguir siendo un nombre.

## Funciones

```js
function damage(amount, knockback = 2) {
    hp -= amount
    hspeed = -knockback * scale_x
    return hp <= 0
}
```

- Los parámetros que faltan valen `null`, o su valor por defecto.
- Una función global se ejecuta "como" quien la llama: dentro, `self` es la instancia que la llamó.
- Una función declarada dentro de un objeto es un **método**: cada instancia tiene el suyo y
  siempre se ejecuta con `self` = esa instancia. Se llama como `damage(1)` desde la propia
  instancia o como `enemy.damage(1)` desde otra.
- Las funciones anónimas (`function(a) { ... }`) también quedan ligadas al `self` donde se crearon.
- Las funciones dentro de un struct quedan ligadas al struct:

```js
var counter = { n: 0, add: function() { n += 1; return n } }
counter.add()
```

### Cierres: las funciones anónimas ven las locales

Una función anónima (o un método de struct, o un `function nombre()` dentro de otra función)
**captura las variables locales** (`var`, parámetros, variables de `for`) del código donde se
crea, así que sirven como callbacks de `filter`, `map`, `sort`…:

```js
var minimo = 3
var fuertes = filter(enemigos, function(e) { return e.hp >= minimo })
var total = 0
map(precios, function(p) { total += p })     // total cambia fuera también
sort(lista, function(a, b) { return dist(x, y, a.x, a.y) - dist(x, y, b.x, b.y) })
```

- Se capturan **por referencia**: la función y el código que la creó comparten la variable. Si
  cualquiera de los dos la cambia, el otro lo ve, también después (`total` arriba). La variable
  sigue viva mientras viva la función aunque la función que la declaró haya terminado:

  ```js
  function contador() { var n = 0; return function() { n += 1; return n } }
  var c = contador()
  c(); c()      // 1, 2: cada contador() crea su propia n
  ```

- `var x` dentro de la función anónima crea una local nueva que tapa la de fuera; los
  parámetros también tapan. Asignar `x = ...` sin `var` a una capturada cambia la de fuera.
- Una función guardada en una variable puede llamarse a sí misma:
  `var fact = function(n) { return n <= 1 ? 1 : n * fact(n - 1) }`.
- Las locales son de toda la función, no del bloque: en un `for (var i = 0; ...)` clásico
  todas las funciones creadas en el bucle comparten la misma `i` (al final valen lo mismo). En
  `for (x in ...)` cada vuelta tiene su propia `x`.
- Además de las locales, siguen viendo las variables de la instancia (`self`) donde se crearon,
  igual que antes.

- Una función puede ejecutarse como **corrutina** (`coroutine(function() { ... wait(10) ... })`):
  se para en cada `wait(pasos)` / `yield()` y sigue en el paso que toca, con sus locales intactas.
  `wait`/`yield` solo valen como instrucción del propio cuerpo (también dentro de `if` y bucles,
  no de `switch`/`with` ni de las funciones a las que llama). Ver
  [funciones.md](funciones.md#corrutinas-y-llamadas-diferidas).

## Objetos

```js
object Slime : Actor {
    // propiedades: se ejecutan en cada instancia nueva, antes de `create`
    // (primero las del padre, luego las del hijo, así el hijo puede sobrescribirlas)
    sprite = slime
    walk_speed = 0.5

    // métodos
    function squash() { destroy() }

    // eventos
    on create { dir = choose(-1, 1) }
    on step   { x += dir * walk_speed }
    on collision(Player) { other.die() }
}
```

### Variables con tipo

Una propiedad puede declarar su tipo con `nombre: tipo = valor`:

```js
object Slime : Actor {
    walk_speed: num(0, 2) = 0.5
    dir: int(-1, 1) = -1
    boss: bool = false
    tint: color = #a7f070
    drops: object = Coin
    mode: list("patrol", "chase") = "patrol"
}
```

| Tipo | Valores | En el editor |
|---|---|---|
| `num`, `num(min, max)` | número (los límites pueden ser `null`) | deslizador si hay límites, si no un campo numérico |
| `int`, `int(min, max)` | número entero | igual, sin decimales |
| `bool` | `true` / `false` | casilla |
| `str` | texto | campo de texto |
| `color` | `#rrggbb` | selector de color |
| `list("a", "b", …)` | uno de esos textos | desplegable |
| `sprite`, `object`, `sound`, `room`, `font` | un asset de ese tipo | desplegable con los assets |
| `path`, `timeline`, `animcurve`, `sequence` | una trayectoria, línea de tiempo, curva o secuencia | desplegable con los assets |

- El motor comprueba el valor de cada instancia nueva **después** de sus propiedades y de las
  variables de la room, y antes de `create`. Un valor fuera de rango es un error con su línea.
- En las rooms los valores se escriben en TOML: los assets por nombre (`drops = "Coin"`) y los
  colores como texto (`tint = "#ff0000"`). Un valor que no encaja impide cargar el proyecto.
- Los hijos heredan el tipo y pueden cambiar el valor con `walk_speed = 1`; no pueden
  redeclararlo con otro tipo. Las variables integradas (`x`, `speed`, `depth`…) no admiten tipo.
- El tipo solo se comprueba al crear la instancia: luego la variable se usa como cualquier otra.

### Eventos

| Evento | Cuándo ocurre |
|---|---|
| `create` | la instancia acaba de crearse (después de sus propiedades) |
| `destroy` | se llama a `destroy()` |
| `step_begin`, `step`, `step_end` | cada paso lógico (60 por segundo por defecto) |
| `alarm(n)` | la alarma `n` (0–11) llega a cero |
| `key(k)`, `key_pressed(k)`, `key_released(k)` | tecla mantenida / recién pulsada / recién soltada. Sin argumento: cualquier tecla |
| `mouse_down(b)`, `mouse_pressed(b)`, `mouse_released(b)` | botón del ratón sobre la caja de la instancia |
| `mouse_enter`, `mouse_leave` | el ratón entra o sale de la caja |
| `global_mouse_down(b)` / `_pressed` / `_released` | botón del ratón en cualquier parte |
| `collision(Objeto)` | la caja de colisión toca a una instancia de `Objeto` (o de sus hijos). `other` es esa instancia |
| `outside_room` | la instancia acaba de salir por completo de la room |
| `boundary` | la instancia acaba de empezar a cruzar el borde de la room |
| `animation_end` | el sprite completó su animación |
| `draw_begin`, `draw`, `draw_end` | dibujo en coordenadas de la room. Si no hay `draw`, se dibuja el sprite automáticamente |
| `draw_gui` | dibujo encima de todo, en coordenadas de pantalla |
| `pre_draw` | antes de `draw_begin`, sobre el fotograma recién borrado, en coordenadas de pantalla |
| `post_draw` | después de `draw_gui`, sobre el fotograma terminado: post-proceso con `draw_surface(application_surface, 0, 0)` y un shader (ver funciones.md) |
| `room_start`, `room_end` | al entrar y al salir de una room |
| `game_start`, `game_end` | al empezar y al cerrar el juego |
| `user(n)` | solo cuando se llama `event_user(n)` (0–15) |
| `network` | algo pasó en la red (alguien se conecta o se va, llega un mensaje o un paquete). Los detalles están en el struct `async_load`; ver [red.md](red.md#el-evento-network) |

Si un hijo define un evento que también tiene su padre, se ejecuta el del hijo. Para ejecutar
también el del padre, llama a `inherited()` dentro del evento.

### Métodos del padre: `inherited()` y `super`

Un hijo puede redefinir un método del padre y seguir usando la versión del padre:

```js
object Enemigo {
    function herir(n) { hp -= n; flash = 6; return hp <= 0 }
    function nombre() { return "enemigo" }
}
object Jefe : Enemigo {
    function herir(n) {
        if escudo > 0 { escudo -= n; return false }
        return inherited()            // = la versión de Enemigo con los mismos argumentos
    }
    function nombre() { return "jefe " + super.nombre() }
    on step { if super.herir(0) { ... } }   // super funciona también en eventos
}
```

- `inherited()` dentro de un **método** llama a la versión del padre de ese mismo método. Sin
  argumentos le pasa los valores actuales de los parámetros; con argumentos, esos. Devuelve lo
  que devuelva. Si ningún padre tiene ese método, no hace nada y devuelve `null`.
- `inherited()` dentro de un **evento** sigue ejecutando el evento del padre, aunque el evento
  llame a métodos que también usan `inherited()`: cada uno va a lo suyo.
- `super.metodo(args)` llama a la versión del padre de cualquier método, desde métodos y
  eventos. Si ningún padre lo tiene es un error (y `modengine check` lo avisa sin ejecutar).
- La búsqueda empieza en el padre del objeto **que declara el código** que se está ejecutando,
  no en el de la instancia: en una cadena `A → B → C`, el `inherited()` del método de B llega
  al de A aunque la instancia sea un C, y un C sin el método usa el de B (cuyo `inherited()`
  sigue llegando a A).
- `self` sigue siendo la misma instancia en todas las versiones.
- No funcionan dentro de funciones anónimas ni de funciones globales (no son de ningún
  objeto); ahí `super.x()` es un error.

Teclas: `key.left`, `key.right`, `key.up`, `key.down`, `key.space`, `key.enter`, `key.escape`,
`key.shift`, `key.ctrl`, `key.alt`, `key.tab`, `key.backspace`, `key.a`…`key.z`, `key.n0`…`key.n9`
(o `key["0"]`), `key.f1`…`key.f12`. Botones: `mouse.left`, `mouse.right`, `mouse.middle`.

## `with`, `self`, `other`, `all`

`with` ejecuta un bloque como si fuese otra instancia (o todas las de un objeto). Dentro,
`self` es esa instancia y `other` es quien ejecutaba el código antes del `with`:

```js
with Slime { walk_speed *= 2 }           // todas las instancias de Slime (y de sus hijos)
with nearest(Enemy) { hp -= other.power }
with all { visible = true }
```

`Objeto.variable` lee de la primera instancia del objeto; `Objeto.variable = v` escribe en todas.

Dentro de un `with` se siguen viendo las variables locales (`var`) del código que lo contiene;
las funciones anónimas también (ver *Cierres*).

## Errores

Los errores de compilación impiden cargar el proyecto y muestran fichero y línea. Los errores en
tiempo de ejecución se imprimen en la consola (una vez cada uno) y aparecen en rojo en la parte
inferior de la ventana; el evento que falló se interrumpe y el juego sigue funcionando.

```text
[error] objects/actors.ms:42: unknown variable 'hp_max' (not set before reading it?)
    in Player.step (objects/actors.ms:30)
```

### Análisis estático en `modengine check`

Antes de ejecutar nada, `modengine check` revisa todo el código (objetos, funciones, código
suelto y código de las rooms) y avisa con fichero y línea:

```text
objects/player.ms:16: aviso: variable desconocida 'sped' (nadie la asigna en el proyecto) (¿quizás quisiste decir 'speed'?)
objects/player.ms:18: error: el objeto 'Enemy' no tiene el método 'hurtt' (¿quizás quisiste decir 'hurt'?)
objects/player.ms:19: error: función o método desconocido 'spwan' (¿quizás quisiste decir 'spawn'?)
objects/player.ms:20: aviso: asset o variable desconocida 'dto' (¿quizás quisiste decir 'dot'?)
objects/player.ms:22: aviso: 'target.hp': 'target' guarda el resultado de una búsqueda de instancias (puede ser null o haberse destruido); compruébalo con exists(target) o `if target` en esta función
static analysis: 2 error(s), 3 warning(s)
```

| Comprobación | Nivel |
|---|---|
| llamar a una función o método que no existe en ningún sitio | error |
| `Objeto.metodo()` / `inst.metodo()` que ese objeto (ni sus padres ni hijos) no tiene | error |
| `super.m()` sin ningún padre que defina `m`, o fuera de un objeto | error |
| leer una variable que nadie asigna en todo el proyecto (también assets mal escritos) | aviso |
| `x += 1` / `x++` de una variable que nunca recibe un valor con `=` | aviso |
| `global.x` que nadie asigna | aviso |
| `Objeto.var` / `inst.var` que ese objeto no tiene | aviso |
| `inst.var` tras `destroy(inst)` en el mismo bloque | aviso |
| `j.x` con `j` de `nearest`/`find`/`inst_at`/`instance_place`… de un objeto cuyas instancias pueden destruirse, sin comprobar `j` (`exists(j)`, `if j != null`, cualquier condición con `j`) en la misma función | aviso |

- Los errores hacen que `check` termine con código 1 (como los errores de ejecución); los
  avisos no.
- Es indulgente a propósito: una variable cuenta como conocida si se asigna **en cualquier
  parte** (cualquier objeto, `with`, campo de struct, `vars` de una room, `set(s, "k", v)`), así
  que casi todo lo que señala es una errata. No sigue el flujo: no sabe si una variable se
  asigna *antes* de leerla.
- El tipo de una instancia solo se conoce cuando viene de buscar un objeto concreto
  (`var j = nearest(Jugador)`, `pa = find(Partida, 0)`); ahí también se comprueban sus
  variables y métodos.
- Una búsqueda solo se avisa si ese objeto puede destruirse en el juego (`destroy()` en su
  código o el de sus padres, `destroy(Objeto)`, `with Objeto destroy()`, `destroy(other)` en
  `collision(Objeto)`). Así los «gestores» que siempre existen no generan avisos.

## Detalles que conviene saber

Comportamientos del motor que no se deducen de los nombres. Cada uno está comprobado por un
test (`crates/modengine/tests/semantics.rs`).

- **Signo de `angle_diff(a, b)`**: devuelve `b − a` llevado a (−180, 180]. Positivo = girar en
  sentido antihorario (hacia «arriba» desde 0, como `angle_to`, donde 90 es arriba). Para
  girar hacia un objetivo: `direction += clamp(angle_diff(direction, objetivo), -giro, giro)`.
  `angle_diff(0, 90)` = 90, `angle_diff(350, 10)` = 20, `angle_diff(0, 180)` = 180.
- **`goto_room(room)` a la misma room la reinicia**: se sale (evento `room_end`), se borran las
  instancias no persistentes y se vuelve a crear todo como al entrar (código de la room,
  `create`, `room_start`). Igual que `restart_room()`. Excepción: si la room es `persistent`,
  al salir se guarda y al entrar se restaura tal cual (mismas instancias, sin `create`).
  El cambio ocurre al terminar el paso, no en el momento de la llamada.
- **Orden de dibujo** (`draw`, `draw_gui` y los demás eventos de dibujo): primero la mayor
  `depth` (más al fondo) y, a igual `depth`, por orden de creación. Las instancias de una room
  se crean en el orden de sus capas y, dentro de cada capa, en el orden en que aparecen en el
  fichero (primero la lista `instances`, luego las colocadas con `map`); las persistentes que vienen de otra room van antes que las nuevas. `draw_gui` usa
  el mismo orden que `draw`.
- **Uniforms**: cada `shader_set_uniform_*` afecta a los dibujos que vienen después, también
  con el mismo shader puesto (no hace falta cambiar de shader ni de material entre dos
  dibujos con valores distintos; ver [funciones.md](funciones.md#shaders-y-modos-de-mezcla)).
- **Cámara**: al final de cada paso la posición se ajusta para no salirse de la room (de 0 a
  `room_width − ancho de vista`); si la room es más pequeña que la vista, queda centrada (con
  `camera_x()` negativo). Vale también para `camera_set`, y `camera_set(x, y)` **deja de
  seguir** al objetivo de `camera_follow` / `[camera] follow` (vuelve a seguir con otro
  `camera_follow`).
- **`vars` de `spawn(obj, x, y, depth, vars)`** (y `vars` de las instancias de una room): se
  asignan **después** de las propiedades del objeto y **antes** de `create`, así que `create`
  ya ve los valores pasados.
- **Locales**: los bloques `with` y las funciones anónimas ven las variables locales del código
  que los contiene (ver *Cierres*).
