param(
    [string]$Workspace = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
$diagnostics = Join-Path $Workspace 'diagnostics'
$samplesRoot = Join-Path $diagnostics 'transform_samples'
$artifacts = Join-Path $Workspace 'artifacts'
[IO.Directory]::CreateDirectory($diagnostics) | Out-Null

function Read-Json([string]$Path) {
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

function Get-SampleHealth([string]$Path) {
    $samples = @(Get-Content -LiteralPath $Path | ForEach-Object { $_ | ConvertFrom-Json })
    $malformed = @($samples | Where-Object malformed).Count
    $withFields = @($samples | Where-Object { $_.decoded_field_count -gt 0 }).Count
    $fullyConsumed = @($samples | Where-Object { $_.parsed_bits -eq $_.payload_bit_count }).Count
    $ratios = @($samples | Where-Object { $_.payload_bit_count -gt 0 } | ForEach-Object {
        [double]$_.parsed_bits / [double]$_.payload_bit_count
    })
    [ordered]@{
        sample_count = $samples.Count
        malformed_count = $malformed
        malformed_ratio = if ($samples.Count) { $malformed / $samples.Count } else { 0 }
        samples_with_decoded_fields = $withFields
        samples_with_decoded_fields_ratio = if ($samples.Count) { $withFields / $samples.Count } else { 0 }
        fully_consumed_count = $fullyConsumed
        fully_consumed_ratio = if ($samples.Count) { $fullyConsumed / $samples.Count } else { 0 }
        mean_parsed_payload_ratio = if ($ratios.Count) { ($ratios | Measure-Object -Average).Average } else { 0 }
    }
}

function Write-Json([string]$Path, $Value) {
    $json = $Value | ConvertTo-Json -Depth 20
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

$globalProbe = Read-Json (Join-Path $artifacts 'probes/ec22cf8e/probe.json')
$globalManifest = Read-Json (Join-Path $artifacts 'global_13_05_baseline_export/manifest.json')
$globalSamplePath = Join-Path $samplesRoot 'global_13_05.jsonl'
$global = [ordered]@{
    schema_version = 1
    experiment = 'unmodified_global_13_05_baseline'
    metadata = $globalProbe
    parser_support = [ordered]@{ supported = $true; transform = 'ValorantSeededTransform13_05' }
    pipeline = $globalManifest.pipeline
    raw_packet_stats = $globalManifest.stats
    decoded_counts = $globalManifest.counts
    transform_sample_health = Get-SampleHealth $globalSamplePath
}
Write-Json (Join-Path $diagnostics 'global_13_05_baseline.json') $global

$fixtures = @(
    [ordered]@{ id='0d7e68dd'; patch='13_05'; file='0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf' },
    [ordered]@{ id='65f18d47'; patch='13_05'; file='65f18d47-82c2-4a75-bf75-edaf780ff0c1.vrf' },
    [ordered]@{ id='a00e078e'; patch='13_05'; file='a00e078e-6412-4785-ada9-45d5f87c1fc1.vrf' },
    [ordered]@{ id='ec9a105c'; patch='13_05'; file='ec9a105c-4fb7-4690-9ad9-29feebd0ea7b.vrf' },
    [ordered]@{ id='5570b968'; patch='13_04'; file='5570b968-df22-4260-bb8b-f300ec312b45.vrf' }
)

foreach ($fixture in $fixtures) {
    $probe = Read-Json (Join-Path $artifacts "probes/$($fixture.id)/probe.json")
    $manifest = Read-Json (Join-Path $artifacts "china_alias_$($fixture.id)/manifest.json")
    $sampleName = "china_$($fixture.patch)_$($fixture.id)_alias_global.jsonl"
    $samplePath = Join-Path $samplesRoot $sampleName
    $summary = [ordered]@{
        schema_version = 1
        experiment = 'china_unmodified_baseline_and_temporary_global_alias'
        metadata = $probe
        unmodified_parser = [ordered]@{
            supported = $false
            expected_error = "no payload transform is registered for replay branch '$($probe.replay.branch)'."
        }
        temporary_alias = [ordered]@{
            selected_transform = if ($fixture.patch -eq '13_05') {
                'ValorantSeededTransformChina13_05DiagnosticAlias -> ValorantSeededTransform13_05'
            } else {
                'ValorantSeededTransformChina13_04DiagnosticAlias -> ValorantSeededTransform13_04'
            }
            reached_eof = $true
            pipeline = $manifest.pipeline
            raw_packet_stats = $manifest.stats
            decoded_counts = $manifest.counts
            transform_sample_health = Get-SampleHealth $samplePath
        }
    }
    Write-Json (Join-Path $diagnostics "china_$($fixture.patch)_$($fixture.id)_baseline.json") $summary
}

$cli = Join-Path $Workspace '.external/ValorantReplayParser/src/CliReader/bin/Release/net10.0/CliReader.exe'
$globalFirst = (Get-Content -LiteralPath $globalSamplePath -TotalCount 1 | ConvertFrom-Json)
$chinaFirstPath = Join-Path $samplesRoot 'china_13_05_0d7e68dd_alias_global.jsonl'
$chinaFirst = (Get-Content -LiteralPath $chinaFirstPath -TotalCount 1 | ConvertFrom-Json)
$firstBlockJson = & $cli transform-first-block `
    $chinaFirst.raw_ciphertext_hex.Substring(0, 16) `
    $chinaFirst.seed `
    --expected $globalFirst.decoded_hex.Substring(0, 16)
if ($LASTEXITCODE -ne 0) {
    throw 'Historical first-block transform probe failed.'
}
[IO.File]::WriteAllText(
    (Join-Path $diagnostics 'first_uint64_historical_transforms.json'),
    ($firstBlockJson -join [Environment]::NewLine) + [Environment]::NewLine,
    [Text.UTF8Encoding]::new($false))
