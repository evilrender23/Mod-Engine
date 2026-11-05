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

## Hoja de ruta propuesta (por valor / esfuerzo)

1. ✅ **Variables con tipo** en objetos (número con rango, booleano, color, lista, asset) y vista
   heredado/sobrescrito en hijos e instancias. Encaja en el formato actual:
   `speed: num(0..10) = 2`.
2. **Operaciones de evento**: copiar/pegar/duplicar/cambiar tipo, y lista de hijos del objeto.
3. **Instancias en room**: escala, rotación, color, nombre y código de creación propio.
4. **Orden de creación** explícito y **herencia de rooms** (`parent = "base"`), muy útil para
   niveles con el mismo HUD/fondo.
5. **Tiles**: línea, selección + mover, flip/rotar por tile y luego autotile (máscara de 16/47).
6. Capas de assets (sprites sueltos sin objeto) y de paths; carpetas de capas.
7. Física y efectos al final: son los que más cambian el motor.

## Cómo repetir el análisis

```bash
G="…/steamapps/common/GameMaker Studio 2"
rea inspect-managed-members "$G/ObjectEditor.dll" --format json > mem-ObjectEditor.json
rea inspect-managed-members "$G/RoomEditor.dll"   --format json > mem-RoomEditor.json
unzip "$G/ProjectTemplates/new_blank/new_blank.yyz"   # formato .yyp / .yy de referencia
```
