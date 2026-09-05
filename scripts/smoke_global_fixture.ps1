[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ReplayPath,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ParserDirectory,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\p0-global')
)

$ErrorActionPreference = 'Stop'

function Get-DotnetCommand {
    $command = Get-Command dotnet -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }

    $defaultPath = 'C:\Program Files\dotnet\dotnet.exe'
    if (Test-Path -LiteralPath $defaultPath) { return $defaultPath }

    throw '.NET SDK was not found. Install the official .NET 10 SDK, or add dotnet.exe to PATH.'
}

$replay = (Resolve-Path -LiteralPath $ReplayPath -ErrorAction Stop).Path
if ([System.IO.Path]::GetExtension($replay) -ne '.vrf') {
    throw "ReplayPath must reference a .vrf file: $replay"
}

$parser = (Resolve-Path -LiteralPath $ParserDirectory -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath (Join-Path $parser 'src\CliReader\CliReader.csproj'))) {
    throw "CliReader project was not found in $parser."
}

$output = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) {
    $existing = Get-ChildItem -LiteralPath $output -Force | Select-Object -First 1
    if ($null -ne $existing) {
        throw "Refusing to overwrite existing P0-GLOBAL output: $output"
    }
} else {
    New-Item -ItemType Directory -Path $output -Force | Out-Null
}

$dotnet = Get-DotnetCommand
$stdout = Join-Path $output 'stdout.log'
$stderr = Join-Path $output 'stderr.log'

Push-Location $parser
try {
    '===== log =====' | Set-Content -LiteralPath $stdout
    '===== log =====' | Set-Content -LiteralPath $stderr
    & $dotnet run --project 'src\CliReader\CliReader.csproj' -- log $replay 1>> $stdout 2>> $stderr
    $logExitCode = $LASTEXITCODE

    '===== export =====' | Add-Content -LiteralPath $stdout
    '===== export =====' | Add-Content -LiteralPath $stderr
    & $dotnet run --project 'src\CliReader\CliReader.csproj' -- export $replay --output $output 1>> $stdout 2>> $stderr
    $exportExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}

[PSCustomObject]@{
    ReplayPath = $replay
    ReplayBytes = (Get-Item -LiteralPath $replay).Length
    ParserDirectory = $parser
    LogExitCode = $logExitCode
    ExportExitCode = $exportExitCode
    EventsPath = (Join-Path $output 'events.ndjson')
    MovementPath = (Join-Path $output 'movement.ndjson')
} | ConvertTo-Json -Depth 2 | Set-Content -LiteralPath (Join-Path $output 'summary.json')

if ($logExitCode -ne 0) { exit $logExitCode }
if ($exportExitCode -ne 0) { exit $exportExitCode }
