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
| `room_start`, `room_end` | al entrar y al salir de una room |
| `game_start`, `game_end` | al empezar y al cerrar el juego |
| `user(n)` | solo cuando se llama `event_user(n)` (0–15) |

Si un hijo define un evento que también tiene su padre, se ejecuta el del hijo. Para ejecutar
también el del padre, llama a `inherited()` dentro del evento.

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

## Errores

Los errores de compilación impiden cargar el proyecto y muestran fichero y línea. Los errores en
tiempo de ejecución se imprimen en la consola (una vez cada uno) y aparecen en rojo en la parte
inferior de la ventana; el evento que falló se interrumpe y el juego sigue funcionando.

```text
[error] objects/actors.ms:42: unknown variable 'hp_max' (not set before reading it?)
    in Player.step (objects/actors.ms:30)
```
