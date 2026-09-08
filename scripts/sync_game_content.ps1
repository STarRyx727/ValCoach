[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$publicRoot = Join-Path $projectRoot 'web\public\game-content'
$agentRoot = Join-Path $publicRoot 'agents'
$abilityRoot = Join-Path $publicRoot 'abilities'
$mapRoot = Join-Path $publicRoot 'maps'
$roleRoot = Join-Path $publicRoot 'roles'
$rankRoot = Join-Path $publicRoot 'ranks'
$catalogUrl = 'https://valorant.dyn.riotcdn.net/x/content-catalog/PublicContentCatalog-release-13.05.zip'

foreach ($directory in @($publicRoot, $agentRoot, $abilityRoot, $mapRoot, $roleRoot, $rankRoot)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

function Save-RemoteAsset {
    param([string]$Uri, [string]$Destination)
    if ([string]::IsNullOrWhiteSpace($Uri)) { return $null }
    if ((Test-Path -LiteralPath $Destination) -and (Get-Item -LiteralPath $Destination).Length -gt 0) {
        return $Destination
    }
    Invoke-WebRequest -Uri $Uri -OutFile $Destination -UseBasicParsing
    return $Destination
}

Write-Host 'Reading Riot Public Content Catalog provenance...'
$riotCatalog = Invoke-WebRequest -Uri $catalogUrl -Method Head -UseBasicParsing
Write-Host 'Reading Valorant-API content indexes...'
$agentResponse = Invoke-RestMethod -Uri 'https://valorant-api.com/v1/agents?isPlayableCharacter=true'
$mapResponse = Invoke-RestMethod -Uri 'https://valorant-api.com/v1/maps'
$tierResponse = Invoke-RestMethod -Uri 'https://valorant-api.com/v1/competitivetiers'

$roles = @{}
$agents = foreach ($agent in ($agentResponse.data | Sort-Object displayName)) {
    $agentIcon = Join-Path $agentRoot "$($agent.uuid).png"
    Save-RemoteAsset -Uri $agent.displayIcon -Destination $agentIcon | Out-Null

    if ($null -ne $agent.role -and -not $roles.ContainsKey($agent.role.uuid)) {
        $roleIcon = Join-Path $roleRoot "$($agent.role.uuid).png"
        Save-RemoteAsset -Uri $agent.role.displayIcon -Destination $roleIcon | Out-Null
        $roles[$agent.role.uuid] = [ordered]@{
            uuid = $agent.role.uuid
            display_name = $agent.role.displayName
            icon = "/game-content/roles/$($agent.role.uuid).png"
        }
    }

    $abilities = foreach ($ability in $agent.abilities) {
        $slot = $ability.slot.ToString().ToLowerInvariant()
        $abilityFile = "$($agent.uuid)-$slot.png"
        if (-not [string]::IsNullOrWhiteSpace($ability.displayIcon)) {
            Save-RemoteAsset -Uri $ability.displayIcon -Destination (Join-Path $abilityRoot $abilityFile) | Out-Null
        }
        [ordered]@{
            slot = $ability.slot
            display_name = $ability.displayName
            icon = if ([string]::IsNullOrWhiteSpace($ability.displayIcon)) { $null } else { "/game-content/abilities/$abilityFile" }
        }
    }

    [ordered]@{
        uuid = $agent.uuid
        developer_name = $agent.developerName
        display_name = $agent.displayName
        role = if ($null -eq $agent.role) { $null } else { $agent.role.displayName }
        icon = "/game-content/agents/$($agent.uuid).png"
        abilities = @($abilities)
    }
}

$maps = foreach ($map in ($mapResponse.data | Where-Object { -not [string]::IsNullOrWhiteSpace($_.mapUrl) } | Sort-Object displayName)) {
    $mapIcon = Join-Path $mapRoot "$($map.uuid).png"
    Save-RemoteAsset -Uri $map.displayIcon -Destination $mapIcon | Out-Null
    [ordered]@{
        uuid = $map.uuid
        display_name = $map.displayName
        developer_name = ($map.mapUrl -split '/')[-1]
        asset_path = $map.mapUrl
        icon = if ([string]::IsNullOrWhiteSpace($map.displayIcon)) { $null } else { "/game-content/maps/$($map.uuid).png" }
    }
}

$latestTierSet = $tierResponse.data | Select-Object -Last 1
$tiers = foreach ($tier in ($latestTierSet.tiers | Where-Object { $_.tierName -and $_.tier -gt 0 } | Sort-Object tier)) {
    $rankFile = "$($tier.tier).png"
    if (-not [string]::IsNullOrWhiteSpace($tier.largeIcon)) {
        Save-RemoteAsset -Uri $tier.largeIcon -Destination (Join-Path $rankRoot $rankFile) | Out-Null
    }
    [ordered]@{
        tier = $tier.tier
        display_name = $tier.tierName
        division_name = $tier.divisionName
        icon = if ([string]::IsNullOrWhiteSpace($tier.largeIcon)) { $null } else { "/game-content/ranks/$rankFile" }
    }
}

$snapshot = [ordered]@{
    schema_version = 1
    generated_at = [DateTime]::UtcNow.ToString('o')
    sources = [ordered]@{
        riot_public_content_catalog = [ordered]@{
            url = $catalogUrl
            release = '13.05'
            etag = $riotCatalog.Headers.ETag
            last_modified = $riotCatalog.Headers.'Last-Modified'
            content_length = [long]$riotCatalog.Headers.'Content-Length'
            purpose = 'Official provenance baseline for names and visual assets'
        }
        valorant_api = [ordered]@{
            agents = 'https://valorant-api.com/v1/agents?isPlayableCharacter=true'
            maps = 'https://valorant-api.com/v1/maps'
            competitive_tiers = 'https://valorant-api.com/v1/competitivetiers'
            purpose = 'Normalized UUID, developer-name, display-name and asset mapping'
        }
    }
    agents = @($agents)
    roles = @($roles.Values | Sort-Object display_name)
    maps = @($maps)
    competitive_tiers = @($tiers)
}

$snapshotPath = Join-Path $publicRoot 'catalog.json'
[System.IO.File]::WriteAllText(
    $snapshotPath,
    ($snapshot | ConvertTo-Json -Depth 20),
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host "Saved offline content snapshot: $snapshotPath"
Write-Host "Agents: $($agents.Count); maps: $($maps.Count); tiers: $($tiers.Count)"
