# Juice, tweens, menús con foco e idiomas

Guía de las piezas de *game feel* y de textos de Mod Engine: animar con **tweens**, dar
respuesta a los golpes (destellos, contornos, números de daño, *hit-stop*, pulsos), navegar
menús con teclado o mando (**foco**), música por capas y **traducir** el juego (tablas de
textos, plurales, pseudolocalización, editor de Idiomas). La lista completa de funciones está
en [funciones.md](funciones.md#tweens-y-easing-tween_).

Todo avanza en el **paso fijo** del motor: con la misma entrada y `--seed` sale exactamente
lo mismo, así que se prueba con `--input`, `--screenshot-at` y `--exec-at`
([pruebas.md](pruebas.md)).

## Tweens

Un tween lleva una o varias variables de su valor actual a otro en un tiempo, con una curva:

```js
// la carta con foco sube 14 px con un pequeño rebote; las demás bajan
tween_to(self, { y: base_y - 14 }, 0.18, "back_out")

// aparecer: desde transparente y 20 px más abajo hasta como está ahora
tween_from(self, { alpha: 0, y: y + 20 }, 0.3, "quad_out", { delay: index * 0.05 })

// una variable suelta, un struct y un color (se mezcla por canal)
tween(self, "angle", 360, 1, "linear")
tween_to(global.hud, { vida_mostrada: vida }, 0.4)
tween_to(self, { tint: #ff4040 }, 0.2, "linear", { yoyo: true })
```

- **Curvas**: `linear`, `smooth` y `<familia>_<in|out|inout>` para `quad`, `cubic`, `quart`,
  `quint`, `sine`, `expo`, `circ`, `back`, `elastic`, `bounce`. `ease("bounce_out", t)` da el
  valor directamente. Por defecto `quad_out`. También vale una **curva de animación** del
  proyecto: `tween_to(self, { x: 200 }, 1, ac_salto)` (usa su primer canal).
- **Opciones**: `{ delay, yoyo, loops, unscaled, on_complete }`. `yoyo` va y vuelve;
  `loops: -1` repite siempre (un «respirar» de UI: `tween_to(self, { scale_x: 1.05, scale_y:
  1.05 }, 0.8, "sine_inout", { yoyo: true, loops: -1 })`).
- **Encadenar**: `tween_chain(a, b, c)` — cada uno empieza cuando acaba el anterior, desde
  donde lo dejó.
- **Cancelar**: `tween_cancel(id)` lo deja donde está; `tween_cancel(id, true)` lo lleva al
  final. `tween_cancel_all(self)` antes de lanzar uno nuevo sobre las mismas variables evita
  que dos tweens se peleen.
- **Esperar en una corrutina**: `wait_until(function() { return tween_done(t) })`.
- **Velocidad de presentación**: `tween_set_speed(2)` acelera todos los tweens (un botón
  «rápido» o «saltar animaciones» con `tween_set_speed(10)`). Los marcados `unscaled` no
  cambian (la UI de pausa). Si usas la cola de acciones ([datos-y-cartas.md](datos-y-cartas.md)),
  pon la misma velocidad en `actionq_set_speed`.

Comparado con las curvas de animación y las secuencias: las **curvas** son la forma (un asset
que se dibuja en el editor), las **secuencias** son animaciones montadas a mano en una línea de
tiempo, y los **tweens** son animaciones de código, cortas y relativas a donde está ahora cada
cosa (subir la carta seleccionada, encoger un botón al pulsarlo).

## Respuesta a los golpes

```js
function recibir_golpe(dano) {
    vida -= dano
    flash(id, #ffffff, 0.12)            // destello blanco que se desvanece
    pulse(id, 1.15, 0.12)               // crece un poco y vuelve
    popup_text(x, y - 16, "-" + str(dano), { color: #ef7d57, scale: 2 })
    hitstop(0.08)                       // congela la acción 5 pasos
    camera_shake(3, 8)
}
```

- `flash`, `sprite_outline` y `sprite_overwrite` los dibuja el **shader de sprites por
  defecto**: no hace falta escribir shaders, no rompen el lote de dibujo (miles de sprites
  siguen en una llamada) y respetan la transparencia del sprite. El contorno es de píxeles
  enteros (1–8), perfecto para pixel art: `sprite_outline(objetivo, #ffcd75)` marca el
  enemigo seleccionado; `sprite_outline(objetivo, null)` lo quita.
- `sprite_overwrite(inst, #8fd3ff, 0.6)` es un tinte fijo (congelado); `#000000, 1` una
  silueta. El destello se pone encima mientras sea más fuerte.
- Si la instancia tiene su propio evento `draw`, llama a `draw_self()` para que se apliquen.
  Con `shader_set` activo no se aplican.
- **Números flotantes**: `popup_text` reutiliza un *pool* de 256 huecos; no hay que destruir
  nada. En coordenadas de room salen encima de todas las instancias; con `gui: true`, en
  pantalla encima de `draw_gui`. Con la fuente integrada a tamaño pequeño, usa `scale: 2` o una
  fuente propia (`font: fnt_numeros`) para que se lean bien.
- **Hit-stop**: `hitstop(s)` congela eventos, movimiento, alarmas, corrutinas, destellos y
  tweens normales; siguen el dibujo, los popups, el audio, el temblor de cámara y los tweens
  `unscaled`. El destello se queda en su punto más fuerte durante la pausa, que es lo que hace
  que el golpe «pese». El contador de pasos (`steps()`, `--screenshot-at`) sigue avanzando.
- **Accesibilidad**: `juice_set_reduced(true)` limita los destellos al 35 %, el temblor de
  cámara al 25 % y quita el «pop» de los números. Ponlo en el menú de opciones (y guárdalo con
  `save_data`).

## Menús con foco (teclado y mando)

```js
// al crear la mano: un rectángulo por carta, en el grupo "mano"
for (var i = 0; i < len(mano); i++) {
    var c = mano[i]
    ui_focus_add("mano", i, c.x - 16, c.y - 20, 32, 40)
}
ui_focus_wrap("mano", true)

on step {
    var f = ui_focus_update()              // flechas / WASD mueven el foco del grupo activo
    if ui_focus_group() == "mano" {
        if ui_focus_confirm() ui_focus_push("enemigos")   // elegir objetivo
    } else {
        if ui_focus_back() ui_focus_pop()                  // Esc: vuelve a la mano, con su foco
        else if ui_focus_confirm() atacar(ui_focus_get("mano"), f)
    }
}
```

- El vecino elegido es el más cercano **en la dirección** (dentro de ±63°); lo de al lado
  cuenta el doble que lo de enfrente, así que en una rejilla se va recto.
- La pila de grupos (`ui_focus_push` / `ui_focus_pop`) resuelve el clásico «cierro el menú y
  no hay nada seleccionado»: el grupo de abajo conserva su foco.
- `ui_focus_rect()` da el rectángulo con foco para dibujar el marco; `ui_focus_at(mouse_x,
  mouse_y)` permite que el ratón cambie también el foco.
- Las teclas se cambian con `ui_focus_keys({ confirm: [key.enter, key.z], back: key.x })`.

## Música por capas

Dos versiones del mismo tema con la misma duración (tranquila y combate) suenan a la vez
empezando en la misma muestra; al entrar en combate solo cambian los volúmenes:

```js
global.musica = audio_play_layers([mus_cueva_calma, mus_cueva_combate], [1, 0])
// al empezar un combate: fundido de 0.8 s sin perder el compás
audio_layers_set(global.musica, [0, 1], 800)
```

## Idiomas

### Tablas

Un fichero por idioma en `lang/`, con el código como nombre:

```toml
# lang/es.toml
title = "Mazmorra"
[menu]
play = "Jugar"                       # clave "menu.play"
greet = "Hola, {name}"
[coins]                              # plural: forma elegida con el argumento n
zero = "Sin monedas"                 # (opcional) cuando n == 0
one = "{n} moneda"
other = "{n} monedas"
```

```toml
# game.toml
[localization]
default = "es"     # idioma base (el del juego al empezar)
fallback = "en"    # si falta una clave en el idioma actual
```

```js
draw_text(160, 8, loc("title"))
draw_text(160, 24, loc("menu.greet", { name: global.jugador }))
draw_text(8, 170, loc("coins", { n: monedas }))
loc_set_language("en")          // desde el menú de opciones
```

- Si falta una clave: se busca en el idioma de reserva, luego en el base y, si no está en
  ninguno, `loc` devuelve la propia clave (se ve enseguida en pantalla) y la apunta en
  `loc_missing()`.
- Plurales con las reglas de cada idioma (CLDR simplificado): español, inglés, alemán,
  italiano… usan `one`/`other`; francés y portugués de Brasil cuentan el 0 como `one`; ruso,
  ucraniano y polaco usan `one`/`few`/`many`; japonés, coreano, chino y turco solo `other`.
- `{{` y `}}` escriben llaves literales.
- Elige el idioma inicial con `loc_system_language()` si existe en `loc_languages()`, y
  guarda la elección del jugador con `save_data`.

### Pseudolocalización

`loc_set_language("pseudo")` (o `loc_set_pseudo(true)` sobre cualquier idioma) muestra los
textos del idioma base **acentuados, un 30 % más largos y entre corchetes**:
`Jugar` → `[Jügár ~~]`. Sirve para encontrar antes de traducir:

- textos que no caben (botones, cartas): el alemán o el ruso suelen ser un 30 % más largos;
- textos escritos a mano en el código (salen sin corchetes);
- textos montados con trozos (`loc("a") + loc("b")` sale como `[..][..]`: mejor una sola
  clave con `{marcadores}`, porque cada idioma ordena la frase a su manera);
- letras que la fuente no tiene.

Pruébalo desde la terminal sin tocar el juego:

```sh
modengine run MiJuego --exec-at '1:loc_set_language("pseudo")' --screenshot-at 60 --no-save
```

### Fuentes y acentos

Las fuentes de `fonts/` solo incluyen los caracteres de sus `ranges` (por defecto ASCII
32–127). Para español, francés, alemán, portugués… añade Latin-1 y Latin Extended-A:
`ranges = [[32, 127], [160, 383]]`. Para ruso o ucraniano, Cirílico (`[1024, 1279]`); para
japonés, chino o coreano hace falta una fuente TTF que los tenga (y mucho atlas: limita los
rangos a los caracteres que use el juego). La pseudolocalización usa letras de Latin-1
(`á é í ö ü ñ ç`), así que también comprueba que la fuente las tenga.

### Comprobar con `modengine check`

```
objects/menu.ms:12: aviso: texto sin traducir: la clave 'menu.options' falta en lang/en.toml
objects/hud.ms:40: error: texto sin traducir: la clave 'hud.gold' falta en lang/en.toml, lang/es.toml
lang/en.toml:0: aviso: 2 clave(s) que no aparecen en el código (ni en loc("clave") ni como texto): old.title, tmp
```

Solo se comprueban las llamadas con la clave escrita tal cual (`loc("menu.play")`). Las claves
que guardas en arrays o structs (`nombres = ["card.fire", ...]`) o como texto en las tablas de
`data/` (`name = "card.fire"` en un registro) cuentan como usadas; las que
se construyen sumando textos (`"card." + id`) no se pueden ver, así que pueden aparecer como
«sin usar».

### El editor: Idiomas

La sección **Idiomas** del navegador de assets lista los idiomas; cualquiera abre la tabla
**claves × idiomas** (Crear → Idioma añade un idioma nuevo):

- las celdas que faltan se marcan en rojo y el contador dice cuántos huecos quedan;
- **＋ Clave** y **＋ Idioma**, buscador (por clave o por texto) y «solo con huecos»;
- los plurales se editan como una línea por forma (`one: {n} moneda`);
- **Exportar CSV** / **Importar CSV**: una fila por clave y una columna por idioma, para pasar
  a traductores u hojas de cálculo; al importar se añaden claves e idiomas nuevos y se
  reemplazan las celdas no vacías;
- Guardar reescribe `lang/*.toml` (se pierden los comentarios de esos ficheros).

El agente del editor sabe que, si el proyecto tiene `lang/`, los textos que ve el jugador van
con `loc()` y las claves nuevas se añaden a todos los idiomas.
