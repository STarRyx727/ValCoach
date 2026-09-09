[CmdletBinding()]
param(
    [switch]$SkipBrowser,
    [switch]$SkipParserSetup
)

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$parserRoot = Join-Path $projectRoot '.external\ValorantReplayParser'
$parserProject = Join-Path $parserRoot 'src\CliReader\CliReader.csproj'
$parserMarker = Join-Path $parserRoot 'VALCOACH_TESTED_COMMIT.txt'
$pinnedParserCommit = 'b51d67423b7b4952d59051cf91e55efa1c42da05'
$parserMarkerVersion = "$pinnedParserCommit|valcoach-cn1305-production-v2"
$webRoot = Join-Path $projectRoot 'web'
$backend = $null
$backendExecutable = Join-Path $projectRoot 'target\debug\valcoach-server.exe'

function Test-LocalTcpPort {
    param([string]$HostName, [int]$Port)
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect($HostName, $Port)
        return $true
    } catch {
        return $false
    } finally {
        $client.Dispose()
    }
}

function Stop-StaleProjectBackend {
    # Closing a console forcibly can prevent PowerShell's finally block from running,
    # leaving the hidden backend alive. Only stop executables whose resolved path is
    # exactly this checkout's build output; never terminate an unrelated port owner.
    $staleProcesses = Get-Process -Name 'valcoach-server' -ErrorAction SilentlyContinue
    foreach ($process in $staleProcesses) {
        try {
            $processPath = [System.IO.Path]::GetFullPath($process.Path)
        } catch {
            continue
        }
        if (-not [string]::Equals(
            $processPath,
            $backendExecutable,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            continue
        }

        Write-Host "Stopping stale ValCoach backend (PID $($process.Id))..."
        Stop-Process -Id $process.Id -Force -ErrorAction Stop
        Wait-Process -Id $process.Id -Timeout 5 -ErrorAction SilentlyContinue
    }
}

foreach ($commandName in @('cargo', 'npm', 'git')) {
    if (-not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        throw "Required command '$commandName' was not found in PATH. See README.md for prerequisites."
    }
}

function Test-ParserReady {
    if (-not (Test-Path -LiteralPath $parserProject) -or -not (Test-Path -LiteralPath $parserMarker)) {
        return $false
    }

    return (Get-Content -LiteralPath $parserMarker -Raw).Trim() -eq $parserMarkerVersion
}

if (-not $SkipParserSetup -and -not (Test-ParserReady)) {
    Write-Host '[1/3] Installing the pinned replay parser (first run only)...'
    & (Join-Path $PSScriptRoot 'setup_parser.ps1') -SkipTests
}
if (-not (Test-ParserReady)) {
    throw "Replay parser was not found at $parserProject. Run scripts\setup_parser.ps1."
}

Stop-StaleProjectBackend
foreach ($port in @(3000, 5173)) {
    if (Test-LocalTcpPort -HostName '127.0.0.1' -Port $port) {
        throw "Port $port is already in use. Close the previous ValCoach instance, then run start.cmd again."
    }
}

if (-not (Test-Path -LiteralPath (Join-Path $webRoot 'node_modules'))) {
    Write-Host '[2/3] Installing web dependencies (first run only)...'
    Push-Location $webRoot
    try {
        & npm ci
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
    } finally {
        Pop-Location
    }
}

try {
    Write-Host '[3/3] Starting ValCoach...'
    Push-Location $projectRoot
    try {
        & cargo build -p valcoach-server
        if ($LASTEXITCODE -ne 0) { throw 'ValCoach backend build failed.' }
    } finally {
        Pop-Location
    }

    if (-not (Test-Path -LiteralPath $backendExecutable)) {
        throw "ValCoach backend executable was not found: $backendExecutable"
    }
    $backend = Start-Process -FilePath $backendExecutable -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
    $ready = $false
    for ($attempt = 0; $attempt -lt 90; $attempt++) {
        if ($backend.HasExited) { throw "ValCoach backend exited with code $($backend.ExitCode)." }
        try {
            $response = Invoke-WebRequest -Uri 'http://127.0.0.1:3000/api/auth/me' -UseBasicParsing -TimeoutSec 1
            $ready = $true
            break
        } catch {
            if ($_.Exception.Response.StatusCode.value__ -eq 401) {
                $ready = $true
                break
            }
            Start-Sleep -Milliseconds 500
        }
    }
    if (-not $ready) { throw 'ValCoach backend did not become ready within 45 seconds.' }

    if (-not $SkipBrowser) {
        Start-Process 'http://127.0.0.1:5173'
    }
    Write-Host 'ValCoach is running at http://127.0.0.1:5173 (Ctrl+C to stop).'
    Push-Location $webRoot
    try {
        & npm run dev -- --host 127.0.0.1 --strictPort
        if ($LASTEXITCODE -ne 0) { throw 'ValCoach web server exited unexpectedly.' }
    } finally {
        Pop-Location
    }
} finally {
    if ($null -ne $backend -and -not $backend.HasExited) {
        Stop-Process -Id $backend.Id -Force -ErrorAction SilentlyContinue
    }
}
