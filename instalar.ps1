# Instala o actualiza Mod Engine en Windows compilándolo desde una rama del repositorio (master por defecto).
# Es el equivalente de instalar.sh.
#
#   git clone https://github.com/evilrender23/Mod-Engine
#   powershell -ExecutionPolicy Bypass -File Mod-Engine\instalar.ps1 [-Repo URL|RUTA] [-Rama master] [-Destino CARPETA]
#
# O en una línea:
#   irm https://raw.githubusercontent.com/evilrender23/Mod-Engine/master/instalar.ps1 | iex
#
# Desde un clon, sin -Repo, instala desde el origin de ese clon.
#
# En <Destino> (por defecto %LOCALAPPDATA%\ModEngine) deja:
#   codigo\    el clon, con la caché de cargo (target\) para que la siguiente compilación sea rápida;
#   app\       modengine.exe, modeditor.exe, el ejemplo platformer y version.json (commit, fecha, rama, repo);
#   registro\  actualizacion.log, con las líneas que lee el editor (PASO n/total, LISTO, APLICADO,
#              ERROR), y compilacion.log, con la salida de git y cargo.
# Añade app\ al PATH del usuario (modengine y modeditor desde cualquier consola) y crea
# «Mod Engine Editor» en el menú Inicio.
#
# Pregunta antes de instalar con winget lo que falte: Git, las Build Tools de Visual Studio con C++ y Rust.
# Actualizaciones: el editor lo relanza con -Actualizar -EsperarPid <su pid> -Relanzar. Así no pregunta
# ni instala requisitos, compila en app.nuevo, escribe LISTO y, cuando el editor se cierra, cambia app\
# por la versión nueva y lo vuelve a abrir.
# Se guarda sin BOM a propósito: `irm | iex` no lo admite.
param(
    [string]$Repo = 'https://github.com/evilrender23/Mod-Engine',
    [string]$Rama = 'master',
    [string]$Destino = (Join-Path $env:LOCALAPPDATA 'ModEngine'),
    [switch]$Actualizar,
    [int]$EsperarPid = 0,
    [switch]$Relanzar
)

# PowerShell 5.1 lee como ANSI un .ps1 sin BOM y estropearía los acentos: se vuelve a leer en UTF-8.
if ('ñ'.Length -ne 1) {
    $relectura = $PSCommandPath
    & ([scriptblock]::Create([IO.File]::ReadAllText($PSCommandPath, [Text.Encoding]::UTF8))) @PSBoundParameters
    exit $LASTEXITCODE
}

$ErrorActionPreference = 'Stop'
$interactivo = -not $Actualizar
if ($Actualizar) {
    # Sin nadie delante, git no debe quedarse esperando credenciales: mejor que falle y lo diga.
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:GCM_INTERACTIVE = 'never'
}
$repoIndicado = [bool]($PSBoundParameters -and $PSBoundParameters.ContainsKey('Repo'))
# Ejecutado desde un clon de Mod Engine y sin -Repo: el mismo repositorio (un fork, una copia local).
$propio = if ($PSCommandPath) { Split-Path $PSCommandPath } elseif ($relectura) { Split-Path $relectura }
if (-not $repoIndicado -and $propio -and (Test-Path (Join-Path $propio 'crates\modengine-editor\Cargo.toml')) -and (Get-Command git -ErrorAction SilentlyContinue)) {
    try {
        $origenPropio = & git -C $propio remote get-url origin 2>$null
        if ($LASTEXITCODE -eq 0 -and $origenPropio) { $Repo = "$origenPropio".Trim() }
    } catch { }
}

$codigo = Join-Path $Destino 'codigo'
$app = Join-Path $Destino 'app'
$nuevo = Join-Path $Destino 'app.nuevo'
$anterior = Join-Path $Destino 'app.anterior'
$registro = Join-Path $Destino 'registro'
$logCompilacion = Join-Path $registro 'compilacion.log'
$log = Join-Path $registro 'actualizacion.log'
# El editor lo deja si se cierra sin aplicar la actualización: se cambia de versión, pero no se abre.
$noRelanzar = Join-Path $registro 'no-relanzar'
$cerrojoRuta = Join-Path $registro 'instalar.lock'
$script:cerrojo = $null
$toolchain = 'stable-x86_64-pc-windows-msvc'
# eframe 0.37 y macroquad piden Rust 1.95 o más nuevo.
$rustMinimo = [version]'1.95'
$TOTAL = 6
$utf8 = New-Object System.Text.UTF8Encoding $false
$script:salidaUltima = @()

function Anotar([string]$linea) {
    [IO.File]::AppendAllText($log, "$linea`r`n", $utf8)
}

function Paso([int]$n, [string]$texto) {
    Anotar "PASO $n/$TOTAL $texto"
    if ($interactivo) { Write-Host "[$n/$TOTAL] $texto" -ForegroundColor Cyan }
}

# Ejecuta un programa con su salida en pantalla y en compilacion.log.
function Ejecutar([string]$exe, [string[]]$argumentos, [string]$carpeta = $codigo, [switch]$SinComprobar) {
    [IO.File]::AppendAllText($logCompilacion, "> $exe $($argumentos -join ' ')`r`n", $utf8)
    $lineas = New-Object System.Collections.Generic.List[string]
    Push-Location $carpeta
    try {
        # Con 'Stop', PowerShell 5.1 corta en la primera línea que el programa escriba en stderr (cargo escribe ahí).
        $ErrorActionPreference = 'Continue'
        & $exe @argumentos 2>&1 | ForEach-Object {
            $texto = "$_"
            $lineas.Add($texto)
            [IO.File]::AppendAllText($logCompilacion, "$texto`r`n", $utf8)
            if ($interactivo) { Write-Host $texto -ForegroundColor DarkGray }
        }
        $salida = $LASTEXITCODE
        $script:salidaUltima = $lineas
    } finally {
        Pop-Location
    }
    if ($salida -ne 0 -and -not $SinComprobar) {
        throw "$exe $($argumentos[0]) terminó con código $salida (detalles en $logCompilacion)"
    }
}

function Tiene([string]$comando) {
    [bool](Get-Command $comando -ErrorAction SilentlyContinue)
}

# Lo recién instalado no está en el PATH de esta consola.
function Refrescar-Path {
    foreach ($ambito in 'Machine', 'User') {
        foreach ($carpeta in ([Environment]::GetEnvironmentVariable('Path', $ambito) -split ';')) {
            if ($carpeta -and -not (($env:Path -split ';') -contains $carpeta)) { $env:Path = "$env:Path;$carpeta" }
        }
    }
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path $cargo) -and -not (($env:Path -split ';') -contains $cargo)) { $env:Path = "$cargo;$env:Path" }
}

function Hay-BuildTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $false }
    [bool](& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
}

function Hay-Rust {
    if (-not (Tiene rustup) -or -not (Tiene cargo)) { return $false }
    if (-not ((& rustup toolchain list) -match "^$toolchain")) { return $false }
    $texto = "$(& rustc "+$toolchain" --version 2>$null)"
    if ($texto -notmatch '^rustc (\d+\.\d+)') { return $false }
    [version]$Matches[1] -ge $rustMinimo
}

function Con-Winget([string]$id, [string[]]$extra = @()) {
    # winget sale con códigos raros aunque instale; lo que vale es que luego se encuentre.
    Ejecutar winget (@('install', '--id', $id, '-e', '--source', 'winget', '--accept-package-agreements', '--accept-source-agreements') + $extra) $Destino -SinComprobar
}

function Instalar-Rust {
    if (-not (Tiene rustup)) { Con-Winget 'Rustlang.Rustup'; Refrescar-Path }
    Ejecutar rustup @('toolchain', 'install', $toolchain, '--profile', 'minimal') $Destino
    Ejecutar rustup @('update', $toolchain) $Destino -SinComprobar
}

# En orden: rustup avisa si no hay Build Tools, y el toolchain MSVC las necesita para enlazar.
$requisitos = @(
    @{ Nombre = 'Git'; Hay = { Tiene git }; Instalar = { Con-Winget 'Git.Git' } },
    @{ Nombre = 'Build Tools de Visual Studio con C++'; Hay = { Hay-BuildTools }
       Instalar = { Con-Winget 'Microsoft.VisualStudio.2022.BuildTools' @('--override', '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended') } },
    @{ Nombre = "Rust $rustMinimo o más nuevo ($toolchain)"; Hay = { Hay-Rust }; Instalar = { Instalar-Rust } }
)

function Comprobar-Requisitos {
    Refrescar-Path
    $faltan = @($requisitos | Where-Object { -not (& $_.Hay) })
    if (-not $faltan) { return }
    $nombres = ($faltan | ForEach-Object { $_.Nombre }) -join ', '
    if (-not $interactivo) { throw "Falta $nombres. Ejecuta instalar.ps1 en una consola para instalarlo." }
    if (-not (Tiene winget)) { throw "Falta $nombres y no hay winget para instalarlo: instálalo a mano y vuelve a ejecutar el instalador." }
    Write-Host "Falta: $nombres" -ForegroundColor Yellow
    $respuesta = Read-Host '¿Instalarlo con winget? [T]odo / [u]no a uno / [n]ada'
    foreach ($requisito in $faltan) {
        if (& $requisito.Hay) { continue }
        $si = switch -regex ($respuesta.Trim().ToLower()) {
            '^(t|todo|s|si|sí)?$' { $true }
            '^u' { (Read-Host "¿Instalar $($requisito.Nombre)? [S/n]") -notmatch '^\s*n' }
            default { $false }
        }
        if (-not $si) { throw "Sin $($requisito.Nombre) no se puede compilar Mod Engine." }
        Write-Host "Instalando $($requisito.Nombre)…"
        & $requisito.Instalar
        Refrescar-Path
        if (-not (& $requisito.Hay)) {
            throw "Se instaló $($requisito.Nombre), pero no aparece: abre una consola nueva y vuelve a ejecutar el instalador."
        }
    }
}

function Explicar-ErrorGit([string]$origen) {
    $texto = $script:salidaUltima -join "`n"
    if ($texto -match 'Authentication failed|Repository not found|could not read Username|\b40[13]\b') {
        throw "Sin acceso a $origen`: hace falta permiso para ver el repositorio y haber iniciado sesión en GitHub con Git o con GitHub Desktop."
    }
    if ($texto -match 'Could not resolve host|unable to access') {
        throw "Sin conexión con $origen."
    }
}

function Descargar-Codigo {
    if (Test-Path (Join-Path $codigo '.git')) {
        if ($repoIndicado) { Ejecutar git @('remote', 'set-url', 'origin', $Repo) }
        $origen = (& git -C $codigo remote get-url origin).Trim()
        try { Ejecutar git @('fetch', '--prune', 'origin', $Rama) } catch { Explicar-ErrorGit $origen; throw }
        Ejecutar git @('reset', '--hard', "origin/$Rama")
        # Sin -x: lo ignorado (target\) es la caché.
        Ejecutar git @('clean', '-fd')
    } else {
        New-Item -ItemType Directory -Force $Destino | Out-Null
        try { Ejecutar git @('clone', '--branch', $Rama, $Repo, $codigo) $Destino } catch { Explicar-ErrorGit $Repo; throw }
    }
}

# No se puede cambiar app\ con el editor o un juego abiertos (Windows bloquea los .exe en uso).
function Esperar-Pid {
    if ($EsperarPid) {
        $proceso = Get-Process -Id $EsperarPid -ErrorAction SilentlyContinue
        if ($proceso) { $proceso.WaitForExit() }
    }
}

function Esperar-Cierre {
    Esperar-Pid
    $avisado = $false
    while (Get-Process modengine, modeditor -ErrorAction SilentlyContinue |
           Where-Object { $_.Path -and $_.Path.StartsWith("$app\", [StringComparison]::OrdinalIgnoreCase) }) {
        if ($interactivo -and -not $avisado) {
            Write-Host 'Cierra el editor y los juegos de Mod Engine abiertos para terminar la instalación…' -ForegroundColor Yellow
            $avisado = $true
        }
        Start-Sleep -Seconds 1
    }
}

# Recién cerrado el editor, Windows o el antivirus pueden tener aún algún fichero abierto.
function Reintentar([scriptblock]$accion) {
    for ($intento = 1; ; $intento++) {
        try { & $accion; return } catch { if ($intento -ge 20) { throw } }
        Start-Sleep -Milliseconds 500
    }
}

function Agregar-PathUsuario([string]$carpeta) {
    $actual = [Environment]::GetEnvironmentVariable('Path', 'User')
    $partes = @($actual -split ';' | Where-Object { $_ })
    if ($partes -notcontains $carpeta) {
        [Environment]::SetEnvironmentVariable('Path', (($partes + $carpeta) -join ';'), 'User')
        return $true
    }
    $false
}

function Crear-Acceso {
    $acceso = Join-Path ([Environment]::GetFolderPath('Programs')) 'Mod Engine Editor.lnk'
    $exe = Join-Path $app 'modeditor.exe'
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($acceso)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $app
    $lnk.IconLocation = "$exe,0"
    $lnk.Description = 'Mod Engine Editor'
    $lnk.Save()
}

function Abrir-Cerrojo {
    try { [IO.File]::Open($cerrojoRuta, 'OpenOrCreate', 'ReadWrite', 'None') } catch { $null }
}

# Si otra instalación tiene el cerrojo (normalmente una que espera a que se cierre el editor), se la
# sustituye: esta compila lo último de la rama y hace lo mismo que ella.
function Tomar-Cerrojo {
    $script:cerrojo = Abrir-Cerrojo
    if ($script:cerrojo) { return }
    $ErrorActionPreference = 'Continue'
    $otras = @(Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe' OR Name = 'pwsh.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessId -ne $PID -and "$($_.CommandLine)" -match 'instalar\.ps1' })
    if (-not $otras) { throw 'Ya hay otra instalación de Mod Engine en marcha.' }
    foreach ($otra in $otras) { & taskkill.exe /PID $otra.ProcessId /T /F 2>&1 | Out-Null }
    for ($i = 0; $i -lt 40 -and -not $script:cerrojo; $i++) {
        Start-Sleep -Milliseconds 500
        $script:cerrojo = Abrir-Cerrojo
    }
    if (-not $script:cerrojo) { throw 'Ya hay otra instalación de Mod Engine en marcha.' }
}

function Principal {
    New-Item -ItemType Directory -Force $registro | Out-Null
    Tomar-Cerrojo
    [IO.File]::WriteAllText($logCompilacion, '', $utf8)
    [IO.File]::WriteAllText($log, '', $utf8)

    Paso 1 'Comprobando Git, las Build Tools de C++ y Rust'
    Comprobar-Requisitos

    Paso 2 "Descargando el código ($Rama)"
    Descargar-Codigo
    $commit = (& git -C $codigo rev-parse HEAD).Trim()
    $version = [ordered]@{
        commit = $commit
        fecha = (& git -C $codigo log -1 --format=%cI).Trim()
        rama = $Rama
        repo = (& git -C $codigo remote get-url origin).Trim()
    }

    Paso 3 'Compilando el motor y el editor (la primera vez tarda unos minutos)'
    Ejecutar cargo @("+$toolchain", 'build', '--release', '--locked', '-p', 'modengine-cli', '-p', 'modengine-editor')
    $compilado = Join-Path $codigo 'target\release'
    foreach ($pieza in 'modengine.exe', 'modeditor.exe') {
        if (-not (Test-Path (Join-Path $compilado $pieza))) { throw "La compilación no dejó $pieza en $compilado" }
    }

    Paso 4 'Preparando la versión nueva'
    if (Test-Path $nuevo) { Reintentar { Remove-Item $nuevo -Recurse -Force } }
    New-Item -ItemType Directory (Join-Path $nuevo 'examples') | Out-Null
    # El editor lanza el juego con el modengine.exe que tenga al lado: van juntos.
    Copy-Item (Join-Path $compilado 'modengine.exe'), (Join-Path $compilado 'modeditor.exe') $nuevo
    # Archivo → Exportar… copia este runner en los juegos exportados para Windows.
    $runner = Join-Path $compilado 'modengine-runner.exe'
    if (Test-Path $runner) { Copy-Item $runner $nuevo }
    Copy-Item (Join-Path $codigo 'examples\platformer') (Join-Path $nuevo 'examples') -Recurse
    Remove-Item (Join-Path $nuevo 'examples\platformer\saves') -Recurse -Force -ErrorAction SilentlyContinue
    [IO.File]::WriteAllText((Join-Path $nuevo 'version.json'), ($version | ConvertTo-Json), $utf8)
    Anotar "LISTO $commit"
    if (Test-Path $app) {
        Paso 5 'Esperando a que se cierre el editor para cambiar de versión'
        Esperar-Cierre
        if (Test-Path $anterior) { Reintentar { Remove-Item $anterior -Recurse -Force } }
        Reintentar { Rename-Item $app 'app.anterior' }
        try { Rename-Item $nuevo 'app' } catch {
            Rename-Item $anterior 'app'
            throw "No se pudo cambiar a la versión nueva ($($_.Exception.Message)); sigue la anterior."
        }
        Remove-Item $anterior -Recurse -Force -ErrorAction SilentlyContinue
    } else {
        Paso 5 "Instalando en $app"
        Move-Item $nuevo $app
    }

    Paso 6 'Añadiendo los comandos al PATH y el editor al menú Inicio'
    $pathNuevo = Agregar-PathUsuario $app
    Crear-Acceso
    Anotar "APLICADO $commit"
    if ($Relanzar -and -not (Test-Path $noRelanzar)) { Start-Process (Join-Path $app 'modeditor.exe') }
    Remove-Item $noRelanzar -ErrorAction SilentlyContinue
    if (-not $interactivo) { return }

    Write-Host "Mod Engine $($commit.Substring(0, 7)) instalado en $app." -ForegroundColor Green
    if ($pathNuevo) { Write-Host 'Abre una consola nueva para usar modengine y modeditor desde cualquier carpeta.' -ForegroundColor Yellow }
    $ejemplo = Join-Path $app 'examples\platformer'
    Write-Host "Pruébalo:  modeditor `"$ejemplo`"    o    modengine run `"$ejemplo`""
}

$resultado = 0
$codificacion = [Console]::OutputEncoding
try {
    # Para leer bien la salida de git y cargo, que escriben en UTF-8.
    try { [Console]::OutputEncoding = $utf8 } catch { }
    Principal
} catch {
    $resultado = 1
    $texto = ($_.Exception.Message -replace '\s*[\r\n]+\s*', ' ').Trim()
    try { Anotar "ERROR $texto" } catch { }
    if ($interactivo) { Write-Host "ERROR: $texto" -ForegroundColor Red }
} finally {
    if ($script:cerrojo) { $script:cerrojo.Dispose() }
    try { [Console]::OutputEncoding = $codificacion } catch { }
}
# Con -File, el código de salida; con `irm | iex`, no se cierra la consola de quien lo ejecuta.
if ($PSCommandPath -or $relectura) { exit $resultado }
