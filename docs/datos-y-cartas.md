# Datos, cartas y cola de acciones

Guía para juegos con mucho contenido (cartas, enemigos, objetos, eventos) y combates por
turnos: el contenido vive en **tablas de datos**, las cartas se mueven entre **mazos**, y la
lógica resuelve al instante mientras una **cola de acciones** reproduce la presentación a la
velocidad que elija el jugador. Todas las funciones están en
[funciones.md](funciones.md#tablas-de-datos-data_).

## Todo es dato

Cada tipo de contenido es una tabla en `data/`: un fichero con una lista de registros, cada uno
con un `id` único. Una carta nueva es un registro nuevo, no código nuevo.

```toml
# data/cards.toml
[[card]]
id = "slash"
name = "Tajo"
cost = 1
kind = "attack"
effects = [{ type = "damage", amount = 6, target = "front" }]

[[card]]
id = "guard"       # los comentarios se conservan al editar en el editor
name = "Guardia"
kind = "skill"
effects = [{ type = "block", amount = 5 }]
```

- El nombre de la tabla es el del fichero (`cards`); el de los registros (`[[card]]`) es libre,
  pero un fichero usa uno solo.
- También valen **JSON** (`[{"id": "bat", "hp": 12}, ...]` o `{"enemies": [...]}`) y **CSV** (la
  primera fila son los nombres de los campos y debe incluir `id`; las celdas con listas o structs
  se escriben en TOML: `"[""fly""]"`, `{ a = 1 }`). Un fichero por tabla.
- En un script: `data_get("cards", "slash")` devuelve una **copia** del registro;
  `data_get("cards", "slash", "cost")` solo un campo; `data_ids`, `data_all`, `data_find` para
  recorrer y buscar.

```js
for (id in data_ids("cards")) {
    var c = data_get("cards", id)
    show_debug_message(c.name + " cuesta " + str(c.cost))
}
var ataques = data_find("cards", "kind", "attack")
var baratas = data_find("cards", function(c) { return c.cost <= 1 })
```

Las cartas de una partida no son los registros: son **instancias** que apuntan a ellos,
`{uid: 7, id: "slash", nivel: 0}`. Así dos copias de la misma carta son distintas (una puede
estar mejorada) y la definición se lee siempre de la tabla.

## Esquema: tipos, valores por defecto y referencias

`data/<tabla>.schema.toml` describe los campos. Con esquema, `modengine check`, el arranque del
juego y el editor comprueban cada registro y señalan los errores con `fichero:línea`.

```toml
# data/cards.schema.toml
[fields]
name = { type = "string", required = true, doc = "Nombre que se ve en la carta" }
cost = { type = "int", default = 1, min = 0, max = 5 }
kind = { type = "enum", values = ["attack", "skill", "power"] }
sprite = { type = "sprite" }
upgrade = { type = "ref", table = "cards" }
effects = { type = "list", of = { type = "struct", tag = "type", variants = {
    damage = { amount = "int", target = { type = "enum", values = ["front", "all"] } },
    block = { amount = "int" },
    draw = { count = "int" },
} } }
```

| Tipo | Valor |
|---|---|
| `string`, `int`, `number`, `bool`, `any` | lo que dicen (`int` admite `3.0`) |
| `enum` | un texto de `values = [...]` |
| `ref` | el id de un registro de `table = "..."` (puede ser la misma tabla) |
| `sprite`, `sound`, `object`, `room`, `font`, `tileset`, `path`, `timeline`, `animcurve`, `sequence`, `particles`, `shader`, `asset` | el nombre de un asset; en el juego llega como el asset (`draw_sprite(c.sprite, ...)`) |
| `list` | `of = "int"` o `of = { ... }` para el tipo de cada elemento |
| `struct` | `fields = { ... }`; con `tag = "type"` y `variants = { nombre = { campos } }` cada valor sigue los campos de su variante (el patrón de efectos) |

Opciones de cada campo: `required`, `default` (se rellena al leer), `min`, `max`, `doc` (el
editor lo enseña al pasar por la cabecera). Un campo que no está en el esquema es un error
(`campo 'cots' no está en data/cards.schema.toml (¿quizás 'cost'?)`), salvo que pongas
`extra_fields = true` arriba del esquema (o dentro de un `struct`).

Lo que se comprueba: TOML/JSON/CSV válidos, registros sin `id` o con `id` repetido, tipos,
`enum`, `min`/`max`, campos obligatorios, campos desconocidos, variantes desconocidas, ids de
otras tablas y nombres de assets que no existen, esquemas sin tabla y tablas definidas dos veces.
Un error impide cargar el juego, igual que un error de sintaxis en un script:

```text
$ modengine check mijuego
could not load the project:
data/cards.toml:7: 'slash': cost = 9 es mayor que el máximo 5
data/cards.toml:15: 'guard': upgrade: 'slahs' no es un id de la tabla 'cards' (¿quizás 'slash'?)
```

## Editar tablas en el editor

**Crear → Tabla de datos** crea `data/<nombre>.toml` con un registro de ejemplo; la sección
**Tablas de datos** del navegador lista las tablas. El editor es una rejilla de registros ×
campos (las columnas salen del esquema, o de los valores si no hay): números con arrastre,
casillas para `bool`, desplegables para `enum` y para las referencias (ids de la otra tabla,
sprites, sonidos…), y TOML en línea para listas y structs. **+ Fila**, ⬆ ⬇ ⧉ 🗑 por fila,
**+ Columna**, búsqueda, y **Crear esquema** escribe un esquema con los tipos deducidos. Los
errores se marcan en rojo en la celda y se listan debajo, con los mismos mensajes que
`modengine check`. Al guardar un TOML se conservan los comentarios y el formato de lo que no
cambió; JSON y CSV se reescriben. Con el juego en marcha (▶ Ejecutar) los cambios guardados se
ven en unos instantes sin reiniciar.

## Efectos componibles

Cada carta es una lista de efectos pequeños (`damage`, `block`, `draw`…) con parámetros; el
código solo implementa el catálogo de efectos. El despachador es un struct que asocia cada
`type` a una función, y `data_dispatch` lo recorre:

```js
global.apply = {
    damage: function(eff, ctx) {
        var targets = eff.target == "all" ? vivos() : [delante()]
        for (en in targets) { en.hp -= max(0, eff.amount - en.block) }
    },
    block: function(eff, ctx) { global.player.block += eff.amount },
    draw: function(eff, ctx) { robar(eff.count) },
}
data_dispatch(data_get("cards", "slash").effects, global.apply, null)
```

Con varios despachadores sobre los mismos datos salen gratis:

- **Proyección**: lo mismo, pero solo calculado, para enseñar el resultado antes de jugar.
- **Descripción generada**: el texto de la carta sale de sus efectos y nunca se desincroniza.

```js
global.preview = {
    damage: function(eff, proj) {
        for (i, en in global.enemies) {
            var hit = eff.target == "all" ? en.hp > 0 : en == delante()
            if hit { proj[i] += max(0, eff.amount - en.block) }
        }
    },
    fallback: function(eff, proj) {},     // los demás efectos no hacen daño
}
global.describe = {
    damage: function(eff) { return str(eff.amount) + " daño" + (eff.target == "all" ? " a todos" : "") },
    block: function(eff) { return str(eff.amount) + " escudo" },
    draw: function(eff) { return "Roba " + str(eff.count) },
}
var texto = join(data_dispatch(def.effects, global.describe), "\n")
```

`fallback` atiende los tipos sin handler propio; sin él, un tipo desconocido es un error con
sugerencia (`no hay handler para el efecto 'damag' (¿quizás 'damage'?)`).

## Mazos: robo, mano, descarte

```js
function nueva_partida(semilla) {
    global.rng = rng_create(semilla)          // un flujo propio: el combate es reproducible
    var cartas = []
    for (id in ["slash", "slash", "slash", "guard", "guard", "cleave"]) {
        global.uid += 1
        push(cartas, {uid: global.uid, id: id})
    }
    global.draw_pile = deck_create(cartas)
    deck_shuffle(global.draw_pile, global.rng)
    global.hand = deck_create()
    global.discard = deck_create()
    global.exhaust = deck_create()            // cartas agotadas: no vuelven en este combate
}

function robar(n) {
    repeat n {
        if deck_size(global.hand) >= 7 { break }               // límite de mano
        if deck_size(global.draw_pile) == 0 {                  // se acabó el mazo: barajar el descarte
            if deck_size(global.discard) == 0 { break }
            deck_move_all(global.discard, global.draw_pile)
            deck_shuffle(global.draw_pile, global.rng)
        }
        var c = deck_move(global.draw_pile, global.hand)
        actionq_push(global.q, anim_robar, c)                   // la vista lo enseñará después
    }
}
```

- Una maldición que mete un enemigo en tu mazo: `deck_add(global.draw_pile, {uid: n, id: "curse"}, deck_random, global.rng)`.
- Una carta con *agotar*: `deck_move(global.hand, global.exhaust, c)` en vez de al descarte.
- **Maná y coste**: comprueba `data_get("cards", c.id, "cost") <= global.player.mana` antes de
  jugar. Para costes raros (X, comodín) guarda la estrategia en los datos (`cost_type = "x"`) y
  despáchala igual que los efectos.
- **Combo**: un contador en el modelo (`global.combo`) que sube si la carta cumple la condición
  (`coste == anterior + 1`…) y que los efectos leen (`eff.amount * (1 + global.combo * 0.1)`).

## Intenciones de los enemigos

La «IA» es una tabla de intenciones en los datos; cada turno el enemigo elige una (con el flujo
de combate, para que sea reproducible) y la vista la enseña encima de él antes de que actúe:

```toml
# data/enemies.toml
[[enemy]]
id = "bat"
hp = 18
sprite = "spr_bat"
intents = [{ type = "attack", amount = 5 }, { type = "block", amount = 4 }]
```

```js
function elegir_intenciones() {
    for (en in vivos()) { en.intent = rng_choose(global.rng, data_get("enemies", en.id).intents) }
}
// en draw_gui, encima del enemigo:
var icono = en.intent.type == "attack" ? ico_attack : ico_block
draw_sprite_ext(icono, 0, en.x - 12, en.y - 58, 3, 3, 0, #ffffff, 1)
draw_text(en.x + 10, en.y - 66, str(en.intent.amount))
```

Fases por vida o por turnos, pesos (`chance`) o intenciones que no se pueden repetir son más
campos en los datos y un `if` al elegir.

## Cola de acciones: la vista va detrás de la lógica

Jugar una carta **resuelve todo al instante** en el modelo (vida, escudo, muertes, robos) y
encola lo que el jugador debe *ver*. La vista dibuja solo lo que las acciones ya han revelado
(`en.shown_hp`, la mano visible), así las animaciones nunca se pisan, la lógica se puede guardar
o probar sin vista y la velocidad de la presentación no cambia los resultados.

```js
// modelo
function jugar(c) {
    var def = data_get("cards", c.id)
    if def.cost > global.player.mana { actionq_push(global.q, anim_texto, "Falta maná"); return }
    global.player.mana -= def.cost
    deck_move(global.hand, global.discard, c)
    actionq_push(global.q, anim_jugar, c)
    data_dispatch(def.effects, global.apply, null)    // los handlers encolan anim_golpe(...)
}

// vista: cada acción es una corrutina
function anim_golpe(en, dmg, hp_final) {
    en.flash = 8
    push(global.numeros, {x: en.x, y: en.y - 40, text: "-" + str(dmg), t: 0})
    camera_shake(2, 6)
    repeat 6 { en.shown_hp = lerp(en.shown_hp, hp_final, 0.5); wait(1) }
    en.shown_hp = hp_final
    wait(4)
}
```

- `actionq_push_parallel(q, [[anim_golpe, a, 6, 12], [anim_golpe, b, 6, 3]])`: un barrido golpea
  a todos a la vez.
- `actionq_push_next(q, anim_explota, en)` desde dentro de una acción: la reacción va justo
  detrás, antes que el resto de la cola.
- `actionq_push_wait(q, 0.3)`: una pausa dramática.
- **Velocidad**: `actionq_set_speed(q, 2)` (×2), `4`, `infinity` (instantáneo, todo en el mismo
  paso y en orden) o `0` (pausa). Un botón que la cambie mantiene el ritmo para quien quiere ir
  deprisa. `actionq_finish(q)` reproduce ya lo pendiente (botón «saltar»).
- `actionq_busy(q)` sirve para no aceptar «Fin de turno» hasta que la vista se ponga al día, si
  el diseño lo pide (la lógica no lo necesita).

### Un turno completo

```js
function empezar_turno() {
    global.turn += 1
    global.player.mana = 3
    global.player.block = 0
    global.state = "player_turn"
    elegir_intenciones()
    robar(5)
}

function fin_de_turno() {
    if global.state != "player_turn" { return }
    global.state = "enemy_turn"
    for (c in deck_to_array(global.hand)) { actionq_push(global.q, anim_descartar, c) }
    deck_move_all(global.hand, global.discard)
    for (en in vivos()) {
        if en.intent.type == "attack" {
            var dmg = max(0, en.intent.amount - global.player.block)
            global.player.hp -= dmg
            actionq_push(global.q, anim_ataque_enemigo, en, dmg)
        } else {
            en.block += en.intent.amount
            actionq_push(global.q, anim_escudo_enemigo, en, en.intent.amount)
        }
    }
    if global.player.hp <= 0 { global.state = "lost"; return }
    empezar_turno()
}

// en el step del objeto de la batalla
if mouse_pressed(mouse.left) {
    var c = carta_en(gui_mouse_x(), gui_mouse_y())
    if c != null { jugar(c) }
}
// en draw_gui: la proyección de la carta bajo el ratón
var proj = array(len(global.enemies), 0)
if hover != null { data_dispatch(data_get("cards", hover.id).effects, global.preview, proj) }
// ... y debajo de cada enemigo: "-12 -> 6"
```

## Máquina de estados

Un turno es una máquina de estados explícita: un único `global.state` (`"player_turn"`,
`"enemy_turn"`, `"reward"`, `"won"`, `"lost"`…) y una función por transición, en vez de banderas
sueltas (`es_mi_turno && !moviendo && ...`). No hace falta nada especial: un texto guardable y
un `switch` en el step.

```js
function cambiar_estado(nuevo) {
    var permitidos = { player_turn: ["enemy_turn", "won"], enemy_turn: ["player_turn", "lost"] }
    if !contains(get(permitidos, global.state, []), nuevo) { error("transición " + global.state + " -> " + nuevo) }
    global.state = nuevo
}

switch global.state {
    case "player_turn": if key_pressed(key.e) { fin_de_turno() }; break
    case "won": if key_pressed(key.enter) { siguiente_combate() }; break
}
```

Como el estado es un texto, guardarlo y una herramienta de depuración «saltar a estado»
(`--exec "global.state = \"reward\""`) son triviales.

## Guardar y cargar una partida

Mazos, el estado del generador aleatorio y la máquina de estados son datos: se guardan juntos
y la partida sigue **exactamente** igual al cargar (mismas cartas en el mismo orden, mismas
tiradas).

```js
function guardar() {
    return save_data("partida", {
        draw: deck_save(global.draw_pile), hand: deck_save(global.hand), discard: deck_save(global.discard),
        rng: rng_get_state(global.rng), player: global.player, enemies: global.enemies,
        state: global.state, turn: global.turn, uid: global.uid,
    })
}

function cargar() {
    var s = load_data("partida", null)
    if s == null { return false }
    actionq_clear(global.q)                  // la presentación pendiente no se guarda
    deck_load(s.draw, global.draw_pile)
    deck_load(s.hand, global.hand)
    deck_load(s.discard, global.discard)
    rng_set_state(global.rng, s.rng)
    global.player = s.player
    global.enemies = s.enemies
    global.state = s.state
    global.turn = s.turn
    global.uid = s.uid
    // la vista se reconstruye del modelo: la mano visible = la mano
    global.view_hand = deck_to_array(global.hand)
    return true
}
```

Guarda ids de la tabla, no registros enteros: si cambias los datos (balance), la partida
guardada usa los valores nuevos. Guarda la cola de acciones nunca: es solo presentación.

## Ejemplo completo

Una batalla pequeña (mano dibujada con `draw_roundrect` y `draw_text`, clic para jugar,
intenciones con iconos, números de daño, proyección al pasar por encima, botón de velocidad
×1/×2/×4/instantáneo y guardado con K / L) cabe en unas 300 líneas: tablas `cards` y
`enemies` con esquema, `scripts/combat.ms` (modelo + acciones) y `objects/battle.ms` (entrada y
dibujo). Pruébala con el ratón simulado:

```sh
modengine run batalla --seed 1 --input "50:mouse=230,300, 70:mb_left" --screenshot-at 62,80,100 --screenshot s_%04d.png
```
