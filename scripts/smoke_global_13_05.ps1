[CmdletBinding()]
param(
    [string]$ReplayPath = (Join-Path $PSScriptRoot '..\Demos-Global\ec22cf8e-b1f4-48b7-8426-c60a20562b3e.vrf'),
    [string]$ParserDirectory = (Join-Path $PSScriptRoot '..\.external\ValorantReplayParser'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\smoke-global-13.05')
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
if ($LASTEXITCODE -ne 0) { throw 'Common container probe failed.' }

dotnet run --no-build --project (Join-Path $parser 'src\CliReader\CliReader.csproj') -- export $replay --profile valcoach --output $parserOutput
if ($LASTEXITCODE -ne 0) { throw 'Global 13.05 payload export failed.' }

Move-Item -LiteralPath (Join-Path $parserOutput 'events.ndjson') -Destination (Join-Path $bundle 'parser_events.ndjson')
Move-Item -LiteralPath (Join-Path $parserOutput 'movement.ndjson') -Destination (Join-Path $bundle 'movement.ndjson')
Move-Item -LiteralPath (Join-Path $parserOutput 'manifest.json') -Destination (Join-Path $bundle 'diagnostics.json')
$probe = Get-Content -LiteralPath (Join-Path $bundle 'probe.json') -Raw | ConvertFrom-Json
$diagnostics = Get-Content -LiteralPath (Join-Path $bundle 'diagnostics.json') -Raw | ConvertFrom-Json
$times = Get-Content -LiteralPath (Join-Path $bundle 'server_events.ndjson') | ForEach-Object { ($_ | ConvertFrom-Json).time_ms }
$manifest = [ordered]@{
    schema_version = 1
    source = $probe.source
    replay = $probe.replay
    backend = [ordered]@{
        name = 'michel-giehl/ValorantReplayParser'
        revision = 'b51d67423b7b4952d59051cf91e55efa1c42da05'
        dialect = 'global-13.05'
        status = 'complete'
        detail = 'Verified Global 13.05 export using the valcoach compact profile'
    }
    validation_backends = @([ordered]@{
        name = 'yakisoba0728/vrfkit:vrf-container'
        revision = 'a73ee3aab474e38af4de7157fb8d94b34bee0963'
        dialect = 'common-container-v7'
        status = 'complete'
        detail = 'Region-independent container and server-event probe'
    })
    capabilities = [ordered]@{
        metadata='complete'; container='complete'; server_events='complete'; movement='complete'
        actors='partial'; player_identity='partial'; gunplay='complete'; combat='partial'
        abilities='partial'; economy='unsupported'; spike_state='partial'; rounds='partial'
        game_state='partial'; world_state='unsupported'; checkpoints='partial'
    }
    records = [ordered]@{
        server_events = [long]$probe.chunks.events
        normalized_events = [long]$diagnostics.counts.events
        movement_samples = [long]$diagnostics.counts.movement
    }
    integrity = [ordered]@{
        malformed_packets = [long]$diagnostics.stats.malformed_packet_count
        partial_errors = [long]$diagnostics.stats.partial_error_count
        undecoded_groups = [long]$diagnostics.counts.undecoded_export_groups
        timeline_coverage_ms = [long](($times | Measure-Object -Maximum).Maximum - ($times | Measure-Object -Minimum).Minimum)
        valid_server_event_payloads = [long]$probe.integrity.valid_event_payloads
        invalid_server_event_payloads = [long]$probe.integrity.invalid_event_payloads
        event_trailing_bytes = [long]$probe.integrity.event_trailing_bytes
        checkpoint_trailing_bytes = [long]$probe.integrity.checkpoint_trailing_bytes
        replay_data_trailing_bytes = [long]$probe.integrity.replay_data_trailing_bytes
    }
    artifacts = @('manifest.json','probe.json','server_events.ndjson','parser_events.ndjson','movement.ndjson','diagnostics.json')
}
$manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $bundle 'manifest.json') -Encoding utf8
python (Join-Path $PSScriptRoot 'summarize_replay_export.py') $bundle --output (Join-Path $output 'summary.json')
if ($LASTEXITCODE -ne 0) { throw 'Replay export summarizer failed.' }
python (Join-Path $PSScriptRoot 'validate_replay_bundle.py') $bundle --output (Join-Path $output 'validation.json')
if ($LASTEXITCODE -ne 0) { throw 'Replay bundle validation failed.' }
Write-Host "Global 13.05 smoke PASS: $bundle"
