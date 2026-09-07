[CmdletBinding()]
param(
    [switch]$SkipBrowser,
    [switch]$SkipParserSetup
)

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$parserRoot = Join-Path $projectRoot '.external\ValorantReplayParser'
$parserProject = Join-Path $parserRoot 'src\CliReader\CliReader.csproj'
$webRoot = Join-Path $projectRoot 'web'
$backend = $null

foreach ($commandName in @('cargo', 'npm', 'git')) {
    if (-not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        throw "Required command '$commandName' was not found in PATH. See README.md for prerequisites."
    }
}

if (-not $SkipParserSetup -and -not (Test-Path -LiteralPath $parserProject)) {
    Write-Host '[1/3] Installing the pinned replay parser (first run only)...'
    & (Join-Path $PSScriptRoot 'setup_parser.ps1') -SkipTests
}
if (-not (Test-Path -LiteralPath $parserProject)) {
    throw "Replay parser was not found at $parserProject. Run scripts\setup_parser.ps1."
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
    $backend = Start-Process -FilePath 'cargo' -ArgumentList @('run', '-p', 'valcoach-server') -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
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
        & npm run dev -- --host 127.0.0.1
    } finally {
        Pop-Location
    }
} finally {
    if ($null -ne $backend -and -not $backend.HasExited) {
        Stop-Process -Id $backend.Id -Force -ErrorAction SilentlyContinue
    }
}
