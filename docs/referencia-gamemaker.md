# Referencia: qué hace el IDE de GameMaker y qué le falta a Mod Engine

Análisis **estático** de GameMaker LTS 2026 (`version.txt` = 2026.0.0.23) instalado en Steam,
hecho con REA (`inspect-managed-artifact` / `inspect-managed-members`) sobre `ObjectEditor.dll`
y `RoomEditor.dll` (.NET 8, IL sin ofuscar), más los JSON y la plantilla `new_blank.yyz` que
trae la instalación. No se ejecutó ni se decompiló código: solo se leyeron nombres de tipos,
métodos, propiedades y literales de texto. Sirve para saber **qué** existe, no **cómo** está
hecho; la implementación de Mod Engine sigue siendo propia.

## Editor de objetos (`ObjectEditor.dll`, 122 tipos)

Observado (tipos y literales):

| GameMaker | Pistas encontradas | Mod Engine hoy |
|---|---|---|
| Eventos con herencia | `ObjectEditor_InheritParentEvent`, `OverrideParentEvent`, `ViewParentEvent`, iconos `event_inherited_icon` / `event_override_icon` | ✅ hereda, *Sobrescribir aquí* con `inherited()` |
| Operaciones de evento | `CopyEvent`, `CutEvent`, `PasteEvent`, `DuplicateEvent`, `ChangeEvent`, `DeleteEvents` (varios a la vez) | ❌ solo añadir/borrar |
| **Variable Definitions** | `GMObjectProperty.varName/varType/filters/listItems`, `rangeEnabled`, `ObjEd_PropertyOptions_`, `eObjectPropertyType` | ✅ `nombre: tipo = valor` (num, int, bool, str, color, list, assets) |
| Sobrescribir variables por hijo / instancia | `InheritedVariableTableRow`, `overriddenProperties`, `Override inherited variable`, `Revoke variable override` | ⚠️ las instancias tienen `vars`, pero sin vista de “heredado / sobrescrito” |
| Árbol de hijos | `ObjectChildrenTreeView`, `Add Child`, `Remove child object`, aviso `This will create a loop in parents` | ⚠️ se ve el padre, no los hijos |
| Máscara | `ObjectEditor_MaskSameAsSprite`, `SelectMask` | ✅ |
| Física | `PhysicsShapeEditor`, `ColShape`, `physicsShapePoints`, `VertexLimitWarning` | ❌ |

## Editor de rooms (`RoomEditor.dll`, 450 tipos)

| GameMaker | Pistas | Mod Engine hoy |
|---|---|---|
| Tipos de capa | `GMRInstanceLayer`, `GMRTileLayer`, `GMRBackgroundLayer`, `GMRAssetLayer`, `GMRPathLayer`, `GMREffectLayer`, `GMRUILayer`, carpetas (`Create Layer Folder`) | ✅ instancias, tiles, fondo · ❌ assets, paths, efectos, UI, carpetas |
| Capa → propiedades | `depth`, `userdefinedDepth`, `visible`, `hierarchyFrozen`, `gridX/gridY`, `effectEnabled` (del `.yy`) | ✅ profundidad/visible · ❌ bloquear capa, rejilla por capa |
| Orden de creación | `InstanceCreationOrder`, `OpenInstanceCreationOrder` | ❌ (orden implícito del mapa) |
| Código de creación | `RoomCreationCode`, `InstanceProperties_CreationCode` (por instancia) | ✅ room · ❌ por instancia |
| Herencia de rooms | `parentRoom`, `inheritLayers`, `inheritCode`, `inheritCreationOrder`, `inheritRoomSettings`, `inheritViewSettings` | ❌ |
| Vistas | `enableViews`, `RoomProperties_View`, `ShowViews`, `clearViewBackground` | ⚠️ una cámara |
| Mundo físico | `PhysicsWorld`, `GravityX/Y`, `PixToMetres` | ❌ |
| Instancia en room | `RoomInstanceInspectorExt`, rotación/escala/color (`, rotation =`, `, scale =`), renombrar instancia | ⚠️ solo x, y, vars |
| Herramientas de tiles | `RoomEditor_PaintBrush`, `TileEraserTool`, `TileFillTool`, `TileLineTool`, `TileRectTool`, `TileSelectionTool`, `TileFlip/Mirror/RotateTool`, `IncreaseBrushSize` | ✅ pintar/borrar/rect/rellenar/cuentagotas · ❌ línea, selección, flip/rotar tile, pinceles de varias celdas |
| Autotile y tiles animados | `RoomEditor_AutoTileTool`, `AutoTileLibrary`, `AnimationLibrary`, `CalculateBitshift` | ❌ |
| Importar mapa de un PNG | `RoomPNG2TilemapTool`, `Generate Tile Map From Image` | ❌ |
| Mover/alinear/duplicar selección | `MoveSelectionUp/Down/Left/Right`, `InvertSelection`, `Duplicate`, `Align canvas item(s)` | ⚠️ mover una instancia |
| Guías | `SmartGuidePrefs`, `GridWidgetSettings_Snap` | ⚠️ ajustar a rejilla |

## Partículas, shaders y extensiones (`ParticleSystemEditor.dll`, `ShaderEditor.dll`, `ExtensionsEditor.dll`)

| GameMaker | Mod Engine |
|---|---|
| Sistema de partículas con emisores que incluyen su tipo (`GMPSEmitterAdapter`, `EmittersPanel`) | ✅ `particles/*.toml` con `[[type]]` y `[[emitter]]` separados (un tipo sirve a varios emisores y a subpartículas) |
| Vista previa en el lienzo (`ParticleCanvas`, `ParticleSysPreviewGadget`), mover emisores (`ParticleEditor_CanvasEmitter`) | ✅ vista previa animada con la simulación del motor, regiones arrastrables y redimensionables |
| Texturas integradas (`InspectorParticleTexSelector`) | ✅ las 14 formas `pt_shape_*` generadas por el motor, o un sprite |
| Presets (`InspectorParticlePresets`) | ✅ menú *Añadir preset* (fuego, humo, chispas, nieve, lluvia, explosión, magia) |
| `part_system_*`, `part_type_*`, `part_emitter_*`, `part_particles_*` | ✅ (ver funciones.md); sin `part_emitter_relative` ni unidades de tiempo en segundos (todo en pasos) |
| Shaders GLSL ES / HLSL / GLSL por plataforma, editor de código del IDE (`GMShaderAdapter`) | ✅ GLSL ES 1.00 con los nombres de GameMaker; ❌ HLSL y GLSL 3 |
| Errores de compilación al ejecutar | ✅ al ejecutar, más comprobación estática en el editor y `modengine shader --check` en la GPU |
| `shader_set_uniform_*`, `texture_set_stage`, `gpu_set_blendmode` | ✅ (sin `gpu_set_blendmode_ext` ni `surface_*`) |
| Extensiones con ficheros nativos DLL/dylib/so/JS por plataforma (`FilePropsWindow`, `AndroidPropsWindow`, `IosPropsWindow`, `HTML5PropsWindow`) | ❌ fuera de alcance: solo ModScript (portátil y seguro) |
| Funciones exportadas con nombre externo y tipos (`FunctionPropsWindow`), constantes (`ConstantPropsWindow`), init/final | ✅ funciones exportadas (documentación + comprobación), constantes, `init`/`final`, orden de ficheros |
| Opciones de extensión editables por proyecto (`ExtensionOptionsWindow`, variables label/number/string/path/boolean/list) | ⚠️ solo constantes (valores TOML) |

## Menú Crear, fuentes, notas y tile sets (`FontEditor.dll`, `NotesEditor.dll`, `TileSetEditor.dll`)

| GameMaker | Mod Engine |
|---|---|
| Menú *Create* con los 15 tipos y atajos Alt | ✅ menú **Crear** (barra, clic derecho en el navegador y en las secciones) con los 15 tipos e iconos; Sprite es Alt+I en vez de Alt+S |
| Fuente: familia del sistema o TTF incluido (`BrowseIncludeTTF`, `GetFontFamilyNames`), tamaño, negrita, cursiva, antialias, rangos (`CreateRangeDialog`, «from text») | ✅ TTF/OTF del proyecto o del sistema (se copia a `fonts/`), negrita/cursiva sintetizadas, antialias, rangos con atajos y «del texto de prueba», vista previa en vivo y página de glifos |
| `font_add_sprite` / `font_add_sprite_ext` | ✅ fuentes de sprite en `fonts/*.toml` (proporcionales o de ancho fijo, espaciado) |
| Editor de glifos (desplazamientos por glifo), SDF (`EnableSDFClicked`), `font_add` en tiempo de ejecución, kerning editable | ❌ (el kerning de la fuente sí se aplica) |
| Notas Markdown (`NotesMarkdownView`) | ✅ texto + vista previa, tareas marcables |
| Tile set: tamaño, desplazamiento, separación, borde de salida | ✅ (sin borde de salida: no hace falta sin texture pages) |
| Animaciones de tiles (`AddNewAnimation`, reordenar frames) | ✅ crear desde la selección, velocidad común y vista previa; ⚠️ los frames se reordenan editando el TOML |
| Auto tiles de 16 y 47 (`Add16TileSet`, `Add47TileSet`), de transición | ⚠️ 16 por vecinos en cruz (arriba/derecha/abajo/izquierda), no la plantilla por esquinas de GameMaker; ❌ 47 y transición |
| Pinceles (`BrushBuilder`), espejo/volteo/giro (X/Y/Z), rellenar, rectángulo | ✅ pincel de varios tiles arrastrando en la paleta, X/Y/Z, rectángulo que repite el pincel, relleno, cuentagotas; ❌ pinceles guardados con nombre, línea, selección + mover |
| `tilemap_*`, `tile_*`, `layer_tilemap_*`, `draw_tile(map)`, `tileset_get_info` | ✅ (ver funciones.md); además `tile_meeting` y `tilemap_autotile`. Diferencia: el vacío es `-1`, no el tile 0 |

## Hoja de ruta propuesta (por valor / esfuerzo)

1. ✅ **Variables con tipo** en objetos (número con rango, booleano, color, lista, asset) y vista
   heredado/sobrescrito en hijos e instancias. Encaja en el formato actual:
   `speed: num(0..10) = 2`.
2. **Operaciones de evento**: copiar/pegar/duplicar/cambiar tipo, y lista de hijos del objeto.
3. **Instancias en room**: escala, rotación, color, nombre y código de creación propio.
4. **Orden de creación** explícito y **herencia de rooms** (`parent = "base"`), muy útil para
   niveles con el mismo HUD/fondo.
5. **Tiles**: ✅ flip/rotar por tile, pinceles de varios tiles, animados y autotile de 16; queda
   línea, selección + mover y autotile de 47.
6. Capas de assets (sprites sueltos sin objeto) y de paths; carpetas de capas.
7. Física y efectos al final: son los que más cambian el motor.

## Cómo repetir el análisis

```bash
G="…/steamapps/common/GameMaker Studio 2"
rea inspect-managed-members "$G/ObjectEditor.dll" --format json > mem-ObjectEditor.json
rea inspect-managed-members "$G/RoomEditor.dll"   --format json > mem-RoomEditor.json
unzip "$G/ProjectTemplates/new_blank/new_blank.yyz"   # formato .yyp / .yy de referencia
```
