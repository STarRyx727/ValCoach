param(
    [string]$ReplayPath = "Demos-China\0d7e68dd-1563-4f12-ba54-1afdf5f99916.vrf",
    [string]$CandidatesFile = "artifacts/final_candidates.txt"
)
$ErrorActionPreference = "Continue"
$root = $PSScriptRoot | Split-Path
$csproj = Join-Path $root ".external\ValorantReplayParser\src\CliReader\CliReader.csproj"
$dotnet = "C:\Program Files\dotnet\dotnet.exe"

$candidates = Get-Content $CandidatesFile | Where-Object { $_ -match '^0x[0-9a-fA-F]+$' }
Write-Host "Testing $($candidates.Count) candidates (env-var driven, single build)"

# Single build
& $dotnet build $csproj --configuration Release 2>&1 | Out-Null
Write-Host "Build done. Starting tests..."

$startTime = Get-Date
$tested = 0
foreach ($sa in $candidates) {
    $tested++
    $saValue = $sa -replace '0x',''
    $tailXor = $saValue.Substring($saValue.Length - 2, 2)

    $env:VALCOACH_SA = $sa
    $env:VALCOACH_IAA = "0x$tailXor"
    $env:VALCOACH_TX = "0x$tailXor"

    $outDir = Join-Path $root "artifacts\env_test"
    Remove-Item $outDir -Recurse -Force -ErrorAction SilentlyContinue
    & $dotnet run --no-build --configuration Release --project $csproj -- export $ReplayPath --profile valcoach --output $outDir *> $null

    $movementFile = Join-Path $outDir "movement.ndjson"
    $eventsFile = Join-Path $outDir "events.ndjson"
    $movementLines = if (Test-Path $movementFile) { (Get-Content $movementFile -ErrorAction SilentlyContinue | Measure-Object -Line).Lines } else { 0 }
    $eventLines = if (Test-Path $eventsFile) { (Get-Content $eventsFile -ErrorAction SilentlyContinue | Measure-Object -Line).Lines } else { 0 }

    $elapsed = ((Get-Date) - $startTime).TotalMinutes
    $eta = if ($tested -gt 1) { ($elapsed / $tested) * ($candidates.Count - $tested) } else { 0 }

    if ($movementLines -gt 0) {
        Write-Host "*** FOUND: SA=$sa movement=$movementLines events=$eventLines ***" -ForegroundColor Green
        break
    } elseif ($eventLines -gt 500) {
        Write-Host "[$tested/$($candidates.Count)] PROMISING: SA=$sa events=$eventLines (eta $([math]::Round($eta,1))min)" -ForegroundColor Yellow
    } else {
        Write-Host "[$tested/$($candidates.Count)] SA=$sa events=$eventLines (eta $([math]::Round($eta,1))min)"
    }
}

$env:VALCOACH_SA = $null
$env:VALCOACH_IAA = $null
$env:VALCOACH_TX = $null
Write-Host "Done."
