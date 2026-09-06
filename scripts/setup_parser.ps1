[CmdletBinding()]
param(
    [string]$ParserDirectory = (Join-Path $PSScriptRoot '..\.external\ValorantReplayParser'),
    [switch]$Refresh,
    [switch]$SkipTests,
    [switch]$ApplyCn1300Alias,
    [switch]$ApplyCn1305Alias
)

$ErrorActionPreference = 'Stop'
$pinnedParserCommit = 'b51d67423b7b4952d59051cf91e55efa1c42da05'

function Get-DotnetCommand {
    $command = Get-Command dotnet -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    $defaultPath = 'C:\Program Files\dotnet\dotnet.exe'
    if (Test-Path -LiteralPath $defaultPath) {
        return $defaultPath
    }

    throw '.NET SDK was not found. Install the official .NET 10 SDK, or add dotnet.exe to PATH.'
}

$dotnet = Get-DotnetCommand
$dotnetVersion = (& $dotnet --version).Trim()
if (-not $dotnetVersion.StartsWith('10.')) {
    throw "ValorantReplayParser requires .NET 10 SDK; found $dotnetVersion."
}

$parserPath = [System.IO.Path]::GetFullPath($ParserDirectory)
$parserParent = Split-Path -Parent $parserPath
if (-not (Test-Path -LiteralPath $parserParent)) {
    New-Item -ItemType Directory -Path $parserParent -Force | Out-Null
}

if (-not (Test-Path -LiteralPath (Join-Path $parserPath '.git'))) {
    if (Test-Path -LiteralPath $parserPath) {
        throw "Parser directory exists but is not a Git checkout: $parserPath"
    }

    git clone --no-checkout https://github.com/michel-giehl/ValorantReplayParser.git $parserPath
    if ($LASTEXITCODE -ne 0) { throw 'Failed to clone ValorantReplayParser.' }
    git -C $parserPath checkout --detach $pinnedParserCommit
    if ($LASTEXITCODE -ne 0) { throw "Failed to check out pinned Parser commit $pinnedParserCommit." }
} elseif (-not (git -C $parserPath rev-parse --verify HEAD 2>$null)) {
    # A clone interrupted before its initial checkout has no HEAD yet. Complete it
    # from the declared official remote instead of using an unversioned snapshot.
    git -C $parserPath fetch origin $pinnedParserCommit
    if ($LASTEXITCODE -ne 0) { throw 'Failed to complete the interrupted Parser clone.' }
    git -C $parserPath checkout --detach $pinnedParserCommit
    if ($LASTEXITCODE -ne 0) { throw 'Failed to check out the fetched Parser commit.' }
} elseif ($Refresh) {
    git -C $parserPath fetch origin $pinnedParserCommit
    if ($LASTEXITCODE -ne 0) { throw 'Failed to refresh the pinned Parser commit.' }
}

$sha = (git -C $parserPath rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Failed to read Parser commit SHA.' }
if ($sha -ne $pinnedParserCommit) {
    throw "Parser is at $sha, but ValCoach requires pinned commit $pinnedParserCommit. Use a clean checkout directory."
}

$valcoachPatch = Join-Path $PSScriptRoot '..\patches\valorant_parser_valcoach_profile.patch'
if (-not (Test-Path -LiteralPath $valcoachPatch)) {
    throw "ValCoach export profile patch was not found: $valcoachPatch"
}

git -C $parserPath apply --reverse --check $valcoachPatch 2>$null
$valcoachPatchAlreadyApplied = $LASTEXITCODE -eq 0
if (-not $valcoachPatchAlreadyApplied) {
    git -C $parserPath apply --check $valcoachPatch
    if ($LASTEXITCODE -ne 0) {
        throw 'ValCoach export profile patch cannot be applied cleanly; use a clean pinned Parser checkout.'
    }
    git -C $parserPath apply $valcoachPatch
    if ($LASTEXITCODE -ne 0) { throw 'Failed to apply the ValCoach export profile patch.' }
    Write-Host 'Applied ValCoach compact export profile.'
} else {
    Write-Host 'ValCoach compact export profile is already applied.'
}

if ($ApplyCn1300Alias) {
    $aliasPatch = Join-Path $PSScriptRoot '..\patches\valorant_parser_cn_13_00_alias.patch'
    if (-not (Test-Path -LiteralPath $aliasPatch)) {
        throw "CN 13.00 alias patch was not found: $aliasPatch"
    }

    git -C $parserPath apply --check $aliasPatch
    if ($LASTEXITCODE -ne 0) {
        throw 'CN 13.00 alias patch cannot be applied cleanly; inspect the pinned Parser checkout before continuing.'
    }

    git -C $parserPath apply $aliasPatch
    if ($LASTEXITCODE -ne 0) { throw 'Failed to apply CN 13.00 alias patch.' }
    Write-Warning 'Applied experimental CN 13.00 alias patch. Validate a real replay before using its output.'
}

if ($ApplyCn1305Alias) {
    $cn1305Patch = Join-Path $PSScriptRoot '..\patches\valorant_parser_cn_13_05_alias.patch'
    git -C $parserPath apply --reverse --check $cn1305Patch 2>$null
    $cn1305AlreadyApplied = $LASTEXITCODE -eq 0
    if (-not $cn1305AlreadyApplied) {
        git -C $parserPath apply --check $cn1305Patch
        if ($LASTEXITCODE -ne 0) {
            throw 'CN 13.05 alias patch cannot be applied cleanly; inspect the pinned Parser checkout.'
        }
        git -C $parserPath apply $cn1305Patch
        if ($LASTEXITCODE -ne 0) { throw 'Failed to apply CN 13.05 alias patch.' }
        Write-Warning 'Applied experimental CN 13.05 alias; it must pass full-file validation before production use.'
    } else {
        Write-Host 'Experimental CN 13.05 alias is already applied.'
    }
}

$sha | Set-Content -LiteralPath (Join-Path $parserPath 'VALCOACH_TESTED_COMMIT.txt') -NoNewline
Write-Host "Parser commit: $sha"
Write-Host "Using .NET SDK: $dotnetVersion"

Push-Location $parserPath
try {
    & $dotnet build 'ValorantReplayParser.sln'
    if ($LASTEXITCODE -ne 0) { throw 'Parser build failed.' }

    if (-not $SkipTests) {
        & $dotnet test 'ValorantReplayParser.sln'
        if ($LASTEXITCODE -ne 0) { throw 'Parser tests failed.' }
    }
} finally {
    Pop-Location
}
