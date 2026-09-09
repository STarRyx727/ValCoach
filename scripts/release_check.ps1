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
$pinnedParserCommit = 'b51d67423b7b4952d59051cf91e55efa1c42da05'
$parserMarkerVersion = "$pinnedParserCommit|valcoach-cn1305-production-v2"

foreach ($commandName in @('cargo', 'dotnet', 'npm', 'node')) {
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

if (-not (Test-Path -LiteralPath $parserMarker -PathType Leaf) -or
    (Get-Content -LiteralPath $parserMarker -Raw).Trim() -ne $parserMarkerVersion) {
    if ($SkipParserSetup) {
        throw "Pinned replay parser is missing: $parserMarker"
    }
    Write-Host '[1/7] Installing the pinned replay parser...'
    & (Join-Path $PSScriptRoot 'setup_parser.ps1') -SkipTests
    if ($LASTEXITCODE -ne 0) { throw 'Parser setup failed.' }
} else {
    Write-Host '[1/7] Pinned replay parser with China 13.05 support is available.'
}

$parserRoot = Join-Path $projectRoot '.external\ValorantReplayParser'
Push-Location $parserRoot
try {
    Write-Host '[2/7] Running parser transform and structural tests...'
    & dotnet test 'tests\Replay.Encoding.Tests\Replay.Encoding.Tests.csproj' --no-build --filter 'FullyQualifiedName~ValorantSeededTransformTests'
    if ($LASTEXITCODE -ne 0) { throw 'Parser transform tests failed.' }
} finally {
    Pop-Location
}

Push-Location $projectRoot
try {
    Write-Host '[3/7] Running the complete Global replay fixture...'
    & cargo test -p valcoach-server jobs::tests::global_13_05_job_reaches_ready_and_persists_a_match_summary -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw 'Global replay fixture failed.' }

    Write-Host '[4/7] Running the complete China 13.05 replay fixture...'
    & cargo test -p valcoach-server jobs::tests::china_13_05_job_imports_full_action_timeline -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw 'China replay fixture failed.' }
} finally {
    Pop-Location
}

Push-Location $webRoot
try {
    Write-Host '[5/7] Installing the exact locked web dependencies...'
    & npm ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }

    Write-Host '[6/7] Running web smoke tests...'
    & npm test
    if ($LASTEXITCODE -ne 0) { throw 'Web smoke tests failed.' }

    Write-Host '[7/7] Building the production web bundle...'
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw 'Web production build failed.' }
} finally {
    Pop-Location
}

Write-Host 'Release check passed: parser transform tests, Global fixture, China fixture, web smoke tests, and web build.'
