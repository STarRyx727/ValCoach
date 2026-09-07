param(
    [string]$ReplayPath = "Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf",
    [string[]]$Candidates = @(
        '0x01726d1c','0x03724d1e','0x05722d20','0x07720d22','0x0972ed13',
        '0x0b72cd15','0x0d72ad17','0x0f728d19','0x11736d0e','0x13734d0c',
        '0x15732d12','0x17730d10','0x1973ed05','0x1b73cd03','0x1d73ad09',
        '0x1f738d07','0x21706d00','0x23704d02','0x25702cfc','0x27700cfe',
        '0x2970ecf7','0x2b70ccf9','0x2d70acf3','0x2f708cf5','0x31716cf2',
        '0x33714cf0','0x35712cee','0x37710cec','0x3971ece9','0x3b71cce7',
        '0x3d71ccfd','0x3f71ecff','0x41746de8','0x43744dea','0x45742dec',
        '0x47740dee','0x4974ede5','0x4b74cde7','0x4d74ade9','0x4f748deb',
        '0x51776d46','0x53774d44','0x55772d42','0x57770d40','0x5977ed37',
        '0x5b77cd35','0x5d77ad33','0x5f778d31','0x61706de0','0x63704de2',
        '0x65702de4','0x67700de6','0x6970eddf','0x6b70cddd','0x6d70addb',
        '0x6f748d3d','0x71716cf2','0x73714cf0','0x75712cee','0x77710cec',
        '0x7971ece9','0x7b71cce7','0x7d71adef','0x7f718ded','0x8172ed9c',
        '0x8372cd9e','0x8572ad90','0x87728d92','0x89726d8b','0x8b740d84',
        '0x8d742d86','0x8f744d80','0x91744d8a','0x93746d88','0x95740d8e',
        '0x97742d8c','0x997bed95','0x9b7acd97','0x9d7bad99','0x9f798d9f',
        '0xa1726d1c','0xa3724d1e','0xa5722d20','0xa7720d22','0xa972ed13',
        '0xab72cd15','0xad72ad17','0xaf728d19','0xb1736d0e','0xb3734d0c',
        '0xb5732d12','0xb7730d10','0xb973ed05','0xbb73cd03','0xbd73ad75',
        '0xbf738d07','0xc1706d00','0xc3704d02','0xc5702cfc','0xc7700dca',
        '0xc970ecf7','0xcb70ccf9','0xcd70acf3','0xcf708cf5','0xd1716cf2',
        '0xd3714cf0','0xd5712cee','0xd7710cec','0xd971ece9','0xdb71cce7',
        '0xdd71ce2f','0xdf7f8ddf','0xe1726e52','0xe3724e50','0xe5722e56',
        '0xe7720e54','0xe972ee5d','0xeb72ce5f','0xed72ae59','0xef728e5b',
        '0xf1706e2c','0xf3704e2e','0xf5702e28','0xf7700e2a','0xf970ee23',
        '0xfb70ce21','0xfd70ae27','0xff700e25'
    )
)
$ErrorActionPreference = "Continue"
$root = $PSScriptRoot | Split-Path
$transformFile = Join-Path $root ".external\ValorantReplayParser\src\Replay.Encoding\PayloadEncryption\VersionedTransforms\ValorantSeededTransform13_05.cs"
$csproj = Join-Path $root ".external\ValorantReplayParser\src\CliReader\CliReader.csproj"
$dotnet = "C:\Program Files\dotnet\dotnet.exe"

Write-Host "Testing $($Candidates.Count) candidates"

foreach ($sa in $Candidates) {
    $saValue = $sa -replace '0x',''
    $tailXor = $saValue.Substring($saValue.Length - 2, 2)
    # IAA from the solution table - use a representative value
    # For now use tail_xor as IAA (they were related in our solutions)

    $content = Get-Content $transformFile -Raw
    $updated = $content -replace 'private const uint SeedAddend = 0x[0-9a-fA-F]+u;', "private const uint SeedAddend = $($sa)u;"
    $updated = $updated -replace 'private const uint InitASeedAddend = 0x[0-9a-fA-F]+u;', "private const uint InitASeedAddend = 0x$($tailXor)u;"
    $updated = $updated -replace 'private const byte TailXor = 0x[0-9a-fA-F]+;', "private const byte TailXor = 0x$($tailXor);"
    Set-Content $transformFile $updated -Encoding UTF8 -NoNewline

    & $dotnet build $csproj --configuration Release 2>&1 | Out-Null

    $outDir = Join-Path $root "artifacts\ct_$($saValue)"
    Remove-Item $outDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $root "content_block_dump.ndjson") -Force -ErrorAction SilentlyContinue
    & $dotnet run --no-build --configuration Release --project $csproj -- export $ReplayPath --profile valcoach --output $outDir *> $null

    $movementFile = Join-Path $outDir "movement.ndjson"
    $eventsFile = Join-Path $outDir "events.ndjson"
    $movementLines = if (Test-Path $movementFile) { (Get-Content $movementFile -ErrorAction SilentlyContinue | Measure-Object -Line).Lines } else { 0 }
    $eventLines = if (Test-Path $eventsFile) { (Get-Content $eventsFile -ErrorAction SilentlyContinue | Measure-Object -Line).Lines } else { 0 }

    if ($movementLines -gt 0) {
        Write-Host "*** FOUND: SA=$sa movement=$movementLines events=$eventLines ***" -ForegroundColor Green
        break
    } elseif ($eventLines -gt 200) {
        Write-Host "PROMISING: SA=$sa events=$eventLines" -ForegroundColor Yellow
    } else {
        Write-Host "SA=$sa events=$eventLines"
    }
}

# Restore
Write-Host "`nRestoring Global constants..."
$content = Get-Content $transformFile -Raw
$restored = $content -replace 'private const uint SeedAddend = 0x[0-9a-fA-F]+u;', "private const uint SeedAddend = 0x48c26613u;"
$restored = $restored -replace 'private const uint InitASeedAddend = 0x[0-9a-fA-F]+u;', "private const uint InitASeedAddend = 0x13u;"
$restored = $restored -replace 'private const byte TailXor = 0x[0-9a-fA-F]+;', "private const byte TailXor = 0x13;"
Set-Content $transformFile $restored -Encoding UTF8 -NoNewline
& $dotnet build $csproj --configuration Release *> $null
Write-Host "Done."
