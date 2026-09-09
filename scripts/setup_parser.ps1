[CmdletBinding()]
param(
    [string]$ParserDirectory,
    [switch]$Refresh,
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ParserDirectory)) {
    # Windows PowerShell 5.1 does not populate $PSScriptRoot while binding
    # default parameter expressions. Resolve the default after binding instead.
    $ParserDirectory = Join-Path $PSScriptRoot '..\.external\ValorantReplayParser'
}
$pinnedParserCommit = 'b51d67423b7b4952d59051cf91e55efa1c42da05'
$parserMarkerVersion = "$pinnedParserCommit|valcoach-cn1305-production-v2"
$gitNetworkOptions = @('-c', 'http.version=HTTP/1.1')

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

$gitProxy = $env:VALCOACH_GIT_PROXY
if ([string]::IsNullOrWhiteSpace($gitProxy) -and (Test-LocalTcpPort '127.0.0.1' 7890)) {
    $gitProxy = 'http://127.0.0.1:7890'
}
if (-not [string]::IsNullOrWhiteSpace($gitProxy)) {
    $gitNetworkOptions += @('-c', "http.proxy=$gitProxy")
    Write-Host "Using Git proxy: $gitProxy"
}

function Invoke-GitNetwork {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureMessage
    )
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $previousErrorAction = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            & git @gitNetworkOptions @Arguments
            $exitCode = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $previousErrorAction
        }
        if ($exitCode -eq 0) { return }
        if ($attempt -lt 3) {
            Write-Warning "Git network operation failed (attempt $attempt/3). Retrying..."
            Start-Sleep -Seconds (2 * $attempt)
        }
    }
    throw "$FailureMessage If GitHub is blocked, start your local proxy or set VALCOACH_GIT_PROXY (for example http://127.0.0.1:7890)."
}

function Test-GitCommand {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & git @Arguments 2>$null
        return $LASTEXITCODE -eq 0
    } catch {
        return $false
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
}

function Invoke-GitCommand {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$FailureMessage
    )
    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & git @Arguments
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    if ($exitCode -ne 0) { throw $FailureMessage }
}

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
        $generatedRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\.external')).TrimEnd('\')
        if (-not $parserPath.StartsWith($generatedRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Parser directory exists but is not a Git checkout: $parserPath"
        }
        Write-Warning "Removing an incomplete generated parser directory from a previous failed install: $parserPath"
        Remove-Item -LiteralPath $parserPath -Recurse -Force
    }

    Invoke-GitNetwork -Arguments @('clone', '--no-checkout', 'https://github.com/michel-giehl/ValorantReplayParser.git', $parserPath) -FailureMessage 'Failed to clone ValorantReplayParser after three attempts.'
    Invoke-GitCommand -Arguments @('-C', $parserPath, 'checkout', '--detach', $pinnedParserCommit) -FailureMessage "Failed to check out pinned Parser commit $pinnedParserCommit."
} elseif (-not (Test-GitCommand -Arguments @('-C', $parserPath, 'rev-parse', '--verify', 'HEAD'))) {
    # A clone interrupted before its initial checkout has no HEAD yet. Complete it
    # from the declared official remote instead of using an unversioned snapshot.
    Invoke-GitNetwork -Arguments @('-C', $parserPath, 'fetch', 'origin', $pinnedParserCommit) -FailureMessage 'Failed to complete the interrupted Parser clone.'
    Invoke-GitCommand -Arguments @('-C', $parserPath, 'checkout', '--detach', $pinnedParserCommit) -FailureMessage 'Failed to check out the fetched Parser commit.'
} elseif ($Refresh) {
    Invoke-GitNetwork -Arguments @('-C', $parserPath, 'fetch', 'origin', $pinnedParserCommit) -FailureMessage 'Failed to refresh the pinned Parser commit.'
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
$valcoachPatchAlreadyApplied = Test-GitCommand -Arguments @('-C', $parserPath, 'apply', '--reverse', '--check', $valcoachPatch)
if (-not $valcoachPatchAlreadyApplied) {
    if (-not (Test-GitCommand -Arguments @('-C', $parserPath, 'apply', '--check', $valcoachPatch))) {
        throw 'ValCoach production patch cannot be applied cleanly; use a clean pinned Parser checkout.'
    }
    Invoke-GitCommand -Arguments @('-C', $parserPath, 'apply', $valcoachPatch) -FailureMessage 'Failed to apply the ValCoach production patch.'
    Write-Host 'Applied the ValCoach profile and dedicated China 13.05 transform.'
} else {
    Write-Host 'ValCoach profile and China 13.05 production transform are already applied.'
}

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

    # The launcher uses this marker to distinguish a complete setup from an
    # interrupted clone, patch, restore or build.
    $parserMarkerVersion | Set-Content -LiteralPath (Join-Path $parserPath 'VALCOACH_TESTED_COMMIT.txt') -NoNewline
} finally {
    Pop-Location
}
