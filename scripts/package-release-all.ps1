#Requires -Version 5.1
param(
    [ValidateSet("gui", "cli", "all")]
    [string]$Target,
    [switch]$Zip
)

$ErrorActionPreference = "Stop"
$settingsPath = Join-Path $PSScriptRoot "release-settings.json"
$settings = Get-Content $settingsPath -Raw | ConvertFrom-Json
$packager = Join-Path $PSScriptRoot "package-release.ps1"

if (-not $Target) { $Target = [string]$settings.defaultTarget }
$zipEnabled = $Zip.IsPresent -or [bool]$settings.createZipByDefault

function Invoke-Packager {
    param(
        [Parameter(Mandatory = $true)][ValidateSet("trial", "self")][string]$Variant,
        [Parameter(Mandatory = $true)][ValidateSet("gui", "cli", "all")][string]$BuildTarget,
        [bool]$CreateZip,
        [int]$TrialDays = 30,
        [string]$TrialExpiresAt
    )

    $packagerArgs = @{
        Variant = $Variant
        Target  = $BuildTarget
    }
    if ($CreateZip) { $packagerArgs.Zip = $true }
    if ($Variant -eq "trial") {
        if ($TrialExpiresAt) {
            $packagerArgs.TrialExpiresAt = $TrialExpiresAt
        } else {
            $packagerArgs.TrialDays = $TrialDays
        }
    }

    & $packager @packagerArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$buildTrial = [bool]$settings.buildTrialByDefault
$buildSelf = [bool]$settings.buildSelfByDefault

if (-not $buildTrial -and -not $buildSelf) {
    Write-Error "release-settings.json: buildTrialByDefault and buildSelfByDefault are both false. Nothing to build."
}

Write-Host "==> SUGT Release All (trial + self)" -ForegroundColor Cyan
Write-Host "    Target : $Target"
Write-Host "    Zip    : $zipEnabled"

if ($buildTrial) {
    Write-Host "`n==> Building trial variant..." -ForegroundColor Yellow
    $trialExpiresAt = if ($settings.defaultTrialExpiresAt) { [string]$settings.defaultTrialExpiresAt } else { $null }
    $trialDays = if ($settings.defaultTrialDays) { [int]$settings.defaultTrialDays } else { 30 }
    Invoke-Packager -Variant trial -BuildTarget $Target -CreateZip $zipEnabled -TrialDays $trialDays -TrialExpiresAt $trialExpiresAt
}

if ($buildSelf) {
    Write-Host "`n==> Building self variant..." -ForegroundColor Yellow
    Invoke-Packager -Variant self -BuildTarget $Target -CreateZip $zipEnabled
}

Write-Host "`n==> All release packages completed." -ForegroundColor Green
