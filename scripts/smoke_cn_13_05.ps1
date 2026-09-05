[CmdletBinding()]
param(
    [string]$ReplayPath = (Join-Path $PSScriptRoot '..\Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf'),
    [string]$ParserDirectory = (Join-Path $PSScriptRoot '..\.external\ValorantReplayParser'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\smoke-china-13.05')
)

$ErrorActionPreference = 'Stop'
$replay = (Resolve-Path -LiteralPath $ReplayPath).Path
$parser = (Resolve-Path -LiteralPath $ParserDirectory).Path
$output = [System.IO.Path]::GetFullPath($OutputDirectory)
$bundle = Join-Path $output 'bundle'
$parserOutput = Join-Path $output 'parser-output'
if (Test-Path -LiteralPath $output) {
    if ($null -ne (Get-ChildItem -LiteralPath $output -Force | Select-Object -First 1)) {
        throw "Refusing to overwrite non-empty smoke output: $output"
    }
} else {
    New-Item -ItemType Directory -Path $output -Force | Out-Null
}
New-Item -ItemType Directory -Path $bundle,$parserOutput -Force | Out-Null

cargo run -p valcoach-vrf-probe -- $replay $bundle
if ($LASTEXITCODE -ne 0) { throw 'China common container probe failed.' }
dotnet run --no-build --project (Join-Path $parser 'src\CliReader\CliReader.csproj') -- export $replay --profile valcoach --output $parserOutput 1> (Join-Path $output 'parser-stdout.log') 2> (Join-Path $output 'parser-stderr.log')
$parserExitCode = $LASTEXITCODE
if ($parserExitCode -eq 0) { throw 'Unexpected China payload success; strict full-file validation is required before enabling it.' }

$probe = Get-Content -LiteralPath (Join-Path $bundle 'probe.json') -Raw | ConvertFrom-Json
$times = Get-Content -LiteralPath (Join-Path $bundle 'server_events.ndjson') | ForEach-Object { ($_ | ConvertFrom-Json).time_ms }
$manifest = [ordered]@{
    schema_version = 1
    source = $probe.source
    replay = $probe.replay
    backend = [ordered]@{
        name = 'michel-giehl/ValorantReplayParser'
        revision = 'b51d67423b7b4952d59051cf91e55efa1c42da05'
        dialect = 'china-13.05'
        status = 'unsupported'
        detail = 'Container and server timeline are valid; no verified China 13.05 payload transform is available'
    }
    validation_backends = @([ordered]@{
        name = 'yakisoba0728/vrfkit:vrf-container'
        revision = 'a73ee3aab474e38af4de7157fb8d94b34bee0963'
        dialect = 'common-container-v7'
        status = 'complete'
        detail = 'Region-independent container and server-event probe'
    })
    capabilities = [ordered]@{
        metadata='complete'; container='complete'; server_events='complete'; movement='unsupported'
        actors='unsupported'; player_identity='unsupported'; gunplay='unsupported'; combat='unsupported'
        abilities='unsupported'; economy='unsupported'; spike_state='partial'; rounds='partial'
        game_state='partial'; world_state='unsupported'; checkpoints='partial'
    }
    records = [ordered]@{ server_events=[long]$probe.chunks.events; normalized_events=0; movement_samples=0 }
    integrity = [ordered]@{
        malformed_packets = $null; partial_errors = $null; undecoded_groups = $null
        timeline_coverage_ms = [long](($times | Measure-Object -Maximum).Maximum - ($times | Measure-Object -Minimum).Minimum)
        valid_server_event_payloads = [long]$probe.integrity.valid_event_payloads
        invalid_server_event_payloads = [long]$probe.integrity.invalid_event_payloads
        event_trailing_bytes = [long]$probe.integrity.event_trailing_bytes
        checkpoint_trailing_bytes = [long]$probe.integrity.checkpoint_trailing_bytes
        replay_data_trailing_bytes = [long]$probe.integrity.replay_data_trailing_bytes
    }
    artifacts = @('manifest.json','probe.json','server_events.ndjson')
}
$manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $bundle 'manifest.json') -Encoding utf8
python (Join-Path $PSScriptRoot 'validate_replay_bundle.py') $bundle --output (Join-Path $output 'validation.json')
if ($LASTEXITCODE -ne 0) { throw 'China container bundle validation failed.' }
Write-Host "China 13.05 container smoke PASS; payload status is unsupported: $bundle"
