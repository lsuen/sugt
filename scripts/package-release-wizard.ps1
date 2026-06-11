#Requires -Version 5.1
$ErrorActionPreference = "Stop"
$packager = Join-Path $PSScriptRoot "package-release.ps1"

Write-Host "SUGT Release Wizard" -ForegroundColor Cyan
$target = Read-Host "Target: gui / cli / all (default: all)"
if ([string]::IsNullOrWhiteSpace($target)) { $target = "all" }
if (@("gui", "cli", "all") -notcontains $target) { throw "Invalid target: $target" }

$variant = Read-Host "Variant: trial / self (default: trial)"
if ([string]::IsNullOrWhiteSpace($variant)) { $variant = "trial" }
if (@("trial", "self") -notcontains $variant) { throw "Invalid variant: $variant" }

$args = @("-Target", $target, "-Variant", $variant)

if ($variant -eq "trial") {
    $expiresAt = Read-Host "Fixed expires at, ISO format, empty means use days"
    if (-not [string]::IsNullOrWhiteSpace($expiresAt)) {
        $args += @("-TrialExpiresAt", $expiresAt)
    } else {
        $days = Read-Host "Trial days (default: 30)"
        if ([string]::IsNullOrWhiteSpace($days)) { $days = "30" }
        $args += @("-TrialDays", $days)
    }
}

$zip = Read-Host "Create zip? y / n (default: y)"
if ([string]::IsNullOrWhiteSpace($zip) -or $zip.ToLowerInvariant().StartsWith("y")) {
    $args += "-Zip"
}

& $packager @args
exit $LASTEXITCODE
