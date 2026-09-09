[CmdletBinding()]
param(
    [switch]$SkipParserSetup
)

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$webRoot = Join-Path $projectRoot 'web'
$parserMarker = Join-Path $projectRoot '.external\ValorantReplayParser\VALCOACH_TESTED_COMMIT.txt'
$globalFixture = Join-Path $projectRoot 'Demos-Global\ec22cf8e-b1f4-48b7-8426-c60a20562b3e.vrf'
$chinaFixture = Join-Path $projectRoot 'Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf'

foreach ($commandName in @('cargo', 'npm', 'node')) {
    if (-not (Get-Command $commandName -ErrorAction SilentlyContinue)) {
        throw "Required command '$commandName' was not found in PATH."
    }
}

$nodeVersion = [version]((& node --version).Trim().TrimStart('v'))
if ($nodeVersion -lt [version]'22.12.0') {
    throw "Node.js 22.12 or newer is required; found $nodeVersion."
}

foreach ($fixture in @($globalFixture, $chinaFixture)) {
    if (-not (Test-Path -LiteralPath $fixture -PathType Leaf)) {
        throw "Release fixture was not found: $fixture"
    }
}

if (-not (Test-Path -LiteralPath $parserMarker -PathType Leaf)) {
    if ($SkipParserSetup) {
        throw "Pinned replay parser is missing: $parserMarker"
    }
    Write-Host '[1/6] Installing the pinned replay parser...'
    & (Join-Path $PSScriptRoot 'setup_parser.ps1') -SkipTests
    if ($LASTEXITCODE -ne 0) { throw 'Parser setup failed.' }
} else {
    Write-Host '[1/6] Pinned replay parser is available.'
}

Push-Location $projectRoot
try {
    Write-Host '[2/6] Running the complete Global replay fixture...'
    & cargo test -p valcoach-server jobs::tests::global_13_05_job_reaches_ready_and_persists_a_match_summary -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw 'Global replay fixture failed.' }

    Write-Host '[3/6] Running the China replay fixture...'
    & cargo test -p valcoach-server jobs::tests::china_13_05_job_imports_common_timeline_and_roster -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw 'China replay fixture failed.' }
} finally {
    Pop-Location
}

Push-Location $webRoot
try {
    Write-Host '[4/6] Installing the exact locked web dependencies...'
    & npm ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }

    Write-Host '[5/6] Running web smoke tests...'
    & npm test
    if ($LASTEXITCODE -ne 0) { throw 'Web smoke tests failed.' }

    Write-Host '[6/6] Building the production web bundle...'
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw 'Web production build failed.' }
} finally {
    Pop-Location
}

Write-Host 'Release check passed: Global fixture, China fixture, web smoke tests, and web build.'
