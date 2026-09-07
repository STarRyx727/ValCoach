param(
    [string]$ReplayPath = "Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf",
    [string[]]$Candidates = @(
        '0x03240ef8','0x008dcd9f','0x0272b473','0x032a8afa','0x004578c2','0x010ac5af',
        '0x01b20b37','0x00f1ed50','0x011a281f','0x02acaf03','0x001c10f2','0x00da5c9a',
        '0x0104ec8a','0x011bfce2','0x01588773','0x01af1ef9','0x02093972','0x026c16b2',
        '0x027ae96e','0x00011f5e'
    )
)
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot | Split-Path
$transformFile = Join-Path $root ".external\ValorantReplayParser\src\Replay.Encoding\PayloadEncryption\VersionedTransforms\ValorantSeededTransform13_05.cs"
$csproj = Join-Path $root ".external\ValorantReplayParser\src\CliReader\CliReader.csproj"
$dotnet = "C:\Program Files\dotnet\dotnet.exe"
$parserRoot = Join-Path $root ".external\ValorantReplayParser"

Write-Host "Testing $($Candidates.Count) candidates against $ReplayPath"

foreach ($sa in $Candidates) {
    $saValue = $sa -replace '0x',''
    $tailXor = $saValue.Substring($saValue.Length - 2, 2)

    Write-Host "`n=== Testing SeedAddend=$sa (TailXor=0x$tailXor) ===" -ForegroundColor Cyan

    # Update the transform file
    $content = Get-Content $transformFile -Raw
    $updated = $content -replace 'private const uint SeedAddend = 0x[0-9a-fA-F]+u;', "private const uint SeedAddend = $($sa)u;"
    $updated = $updated -replace 'private const uint InitASeedAddend = 0x[0-9a-fA-F]+u;', "private const uint InitASeedAddend = 0x$($tailXor)u;"
    $updated = $updated -replace 'private const byte TailXor = 0x[0-9a-fA-F]+;', "private const byte TailXor = 0x$($tailXor);"
    Set-Content $transformFile $updated -Encoding UTF8 -NoNewline

    # Build
    $buildResult = & $dotnet build $csproj --configuration Release 2>&1 | Select-String -Pattern "error|已成功|Build succeeded"
    if ($buildResult -match "error") {
        Write-Host "  BUILD FAILED" -ForegroundColor Red
        continue
    }

    # Run parser on China replay
    $outDir = Join-Path $root "artifacts\china_test_$($saValue)"
    Remove-Item $outDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $parserRoot "content_block_dump.ndjson") -Force -ErrorAction SilentlyContinue
    $result = & $dotnet run --no-build --configuration Release --project $csproj -- export $ReplayPath --profile valcoach --output $outDir 2>&1 | Out-String

    # Check movement lines
    $movementFile = Join-Path $outDir "movement.ndjson"
    $movementLines = if (Test-Path $movementFile) { (Get-Content $movementFile | Measure-Object -Line).Lines } else { 0 }
    $eventsFile = Join-Path $outDir "events.ndjson"
    $eventLines = if (Test-Path $eventsFile) { (Get-Content $eventsFile | Measure-Object -Line).Lines } else { 0 }

    $exitCode = $LASTEXITCODE
    Write-Host "  exit=$exitCode events=$eventLines movement=$movementLines"

    if ($movementLines -gt 0) {
        Write-Host "  *** FOUND IT! SeedAddend=$sa produces $movementLines movement lines ***" -ForegroundColor Green
        break
    } elseif ($eventLines -gt 200) {
        Write-Host "  *** PROMISING: $eventLines events (baseline ~85) ***" -ForegroundColor Yellow
    }
}

# Restore Global constants
Write-Host "`n=== Restoring Global constants ==="
$content = Get-Content $transformFile -Raw
$restored = $content -replace 'private const uint SeedAddend = 0x[0-9a-fA-F]+u;', "private const uint SeedAddend = 0x48c26613u;"
$restored = $restored -replace 'private const uint InitASeedAddend = 0x[0-9a-fA-F]+u;', "private const uint InitASeedAddend = 0x13u;"
$restored = $restored -replace 'private const byte TailXor = 0x[0-9a-fA-F]+;', "private const byte TailXor = 0x13;"
Set-Content $transformFile $restored -Encoding UTF8 -NoNewline
& $dotnet build $csproj --configuration Release | Out-Null
Write-Host "Done."
