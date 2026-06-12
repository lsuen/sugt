#Requires -Version 5.1
<#
.SYNOPSIS
  Build SUGT GUI/CLI and assemble a portable package under release/.

.EXAMPLE
  .\scripts\package-release.ps1 -Variant trial -TrialDays 30 -Zip
  .\scripts\package-release.ps1 -Variant trial -TrialExpiresAt "2026-07-10T23:59:59+08:00" -Zip
  .\scripts\package-release.ps1 -Variant self -Zip
  .\scripts\package-release.ps1 -Product store -Variant self -Zip
  .\scripts\package-release.ps1 -SkipBuild -Variant self
#>
param(
    [ValidateSet("feature", "store")]
    [string]$Product = "feature",
    [ValidateSet("trial", "self")]
    [string]$Variant = "self",
    [ValidateSet("gui", "cli", "all")]
    [string]$Target = "all",
    [int]$TrialDays = 30,
    [string]$TrialExpiresAt,
    [switch]$SkipBuild,
    [switch]$Zip,
    [switch]$Test
)

$ErrorActionPreference = "Stop"
$OutputEncoding = New-Object System.Text.UTF8Encoding $false
$utf8NoBom = New-Object System.Text.UTF8Encoding $false

$RootDir = Split-Path $PSScriptRoot -Parent
$TauriDir = Join-Path $RootDir "src-tauri"
$ReleaseRoot = Join-Path $RootDir "release"
$TargetDir = Join-Path $TauriDir "target\release"

function Test-CommandExists {
    param([Parameter(Mandatory = $true)][string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Assert-CommandExists {
    param([Parameter(Mandatory = $true)][string]$Name)
    if (-not (Test-CommandExists $Name)) {
        throw "Command not found: $Name. Please install it and add it to PATH."
    }
}

function Read-Version {
    $cargo = Join-Path $TauriDir "Cargo.toml"
    $content = [System.IO.File]::ReadAllText($cargo)
    if ($content -match '(?m)^version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
    throw "Cannot read version from Cargo.toml."
}

function Ensure-VcEnv {
    $candidates = @(
        "C:\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) { return $candidate }
    }

    return $null
}

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @(),
        [string]$WorkingDirectory = $RootDir,
        [string]$DisplayName = $FilePath
    )

    Push-Location $WorkingDirectory
    try {
        & $FilePath @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$DisplayName failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

function Invoke-Cargo {
    param([Parameter(Mandatory = $true)][string[]]$CargoArgs)

    $vcvars = Ensure-VcEnv
    if ($vcvars) {
        $escapedArgs = [string]::Join(' ', $CargoArgs)
        cmd /d /c "`"$vcvars`" && cargo $escapedArgs"
        if ($LASTEXITCODE -ne 0) { throw "cargo $escapedArgs failed with exit code $LASTEXITCODE" }
    } else {
        & cargo @CargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo $([string]::Join(' ', $CargoArgs)) failed with exit code $LASTEXITCODE" }
    }
}

function Assert-FileExists {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path $Path)) {
        throw "Missing $Label at $Path. Run a full build without -SkipBuild."
    }
}

function Copy-ReleaseFile {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    Copy-Item $Source $Destination -Force
    if (-not (Test-Path $Destination)) {
        throw "Copy failed: $Destination"
    }
}

function Get-RunningReleaseProcesses {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $normalized = [System.IO.Path]::GetFullPath($Directory)
    $matches = @()

    try {
        $processes = Get-CimInstance Win32_Process -Filter "Name = 'SUGT.exe' OR Name = 'sugt-cli.exe'" -ErrorAction Stop
        foreach ($process in $processes) {
            if ($process.ExecutablePath) {
                $processPath = [System.IO.Path]::GetFullPath($process.ExecutablePath)
                if ($processPath.StartsWith($normalized, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $matches += "{0} (PID {1}, path {2})" -f $process.Name, $process.ProcessId, $processPath
                }
            }
        }
    } catch {
        return @()
    }

    return $matches
}

function Clear-DirectoryContents {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path $Path)) { return }

    $lastError = $null
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Get-ChildItem $Path -Force | Remove-Item -Recurse -Force -ErrorAction Stop
            return
        } catch {
            $lastError = $_
            if ($attempt -lt 3) { Start-Sleep -Seconds 1 }
        }
    }

    $running = Get-RunningReleaseProcesses -Directory $Path
    $hint = "Close any running SUGT.exe or sugt-cli.exe from the release output directory and retry."
    if ($running.Count -gt 0) {
        $hint = "$hint Running process: $($running -join '; ')"
    }

    throw "Cannot clean release output directory: $Path. $hint Original error: $($lastError.Exception.Message)"
}

function Resolve-TrialExpiresAt {
    if ($Variant -eq "self") { return $null }

    if ($TrialExpiresAt) {
        try {
            return ([DateTimeOffset]::Parse($TrialExpiresAt)).ToString("o")
        } catch {
            throw "Invalid -TrialExpiresAt value: $TrialExpiresAt. Use an ISO time like 2026-07-10T23:59:59+08:00."
        }
    }

    if ($TrialDays -le 0) {
        throw "-TrialDays must be greater than 0."
    }

    return (Get-Date).Date.AddDays($TrialDays).AddSeconds(-1).ToString("o")
}

function Set-BuildMetadata {
    param(
        [Parameter(Mandatory = $true)][string]$BuildId,
        [string]$ExpiresAt
    )

    $env:SUGT_BUILD_ID = $BuildId
    $env:SUGT_PRODUCT = $Product
    $env:SUGT_EDITION = $Variant
    if ($Variant -eq "trial") {
        $env:SUGT_TRIAL_ENABLED = "true"
        $env:SUGT_TRIAL_EXPIRES_AT = $ExpiresAt
    } else {
        $env:SUGT_TRIAL_ENABLED = "false"
        Remove-Item Env:SUGT_TRIAL_EXPIRES_AT -ErrorAction SilentlyContinue
    }
}

function Clear-BuildMetadata {
    Remove-Item Env:SUGT_BUILD_ID -ErrorAction SilentlyContinue
    Remove-Item Env:SUGT_PRODUCT -ErrorAction SilentlyContinue
    Remove-Item Env:SUGT_EDITION -ErrorAction SilentlyContinue
    Remove-Item Env:SUGT_TRIAL_ENABLED -ErrorAction SilentlyContinue
    Remove-Item Env:SUGT_TRIAL_EXPIRES_AT -ErrorAction SilentlyContinue
}

function Target-IncludesGui { return $Target -eq "gui" -or $Target -eq "all" }
function Target-IncludesCli { return $Target -eq "cli" -or $Target -eq "all" }

Write-Host "==> SUGT Release Packager" -ForegroundColor Cyan

$version = Read-Version
$buildId = Get-Date -Format "yyyyMMdd-HHmmss"
$expiresAt = Resolve-TrialExpiresAt
$packageName = if ($Product -eq "store") {
    "SUGT-$version-store-$Variant-windows-x64"
} else {
    "SUGT-$version-$Variant-windows-x64"
}
$outDir = Join-Path $ReleaseRoot $packageName

Write-Host "    Version : $version"
Write-Host "    Product : $Product"
Write-Host "    Variant : $Variant"
Write-Host "    Target  : $Target"
Write-Host "    BuildId : $buildId"
if ($expiresAt) { Write-Host "    Expires : $expiresAt" }
Write-Host "    Output  : $outDir"

try {
    Set-BuildMetadata -BuildId $buildId -ExpiresAt $expiresAt

    if ($SkipBuild) {
        Write-Host ""
        Write-Host "WARNING: -SkipBuild skips recompilation. Embedded product/trial/self metadata in existing binaries may NOT match -Product / -Variant / -TrialExpiresAt for this run." -ForegroundColor Yellow
        Write-Host "         Package labels (VERSION.txt, README) will reflect current script params, but exe built-in edition/expiry may differ." -ForegroundColor Yellow
        Write-Host "         For trial/self smoke tests, run a full build without -SkipBuild unless you are certain binaries are fresh." -ForegroundColor Yellow
        Write-Host ""
    }

    if (-not $SkipBuild) {
        Assert-CommandExists "npm"
        Assert-CommandExists "cargo"

        if (Target-IncludesGui) {
            Write-Host "`n==> [1/3] Frontend (npm run build)..." -ForegroundColor Yellow
            Invoke-CheckedCommand -FilePath "npm" -Arguments @("run", "build") -WorkingDirectory $RootDir -DisplayName "npm run build"

            Write-Host "`n==> [2/3] GUI (npx tauri build --no-bundle)..." -ForegroundColor Yellow
            Invoke-CheckedCommand -FilePath "npx" -Arguments @("tauri", "build", "--no-bundle") -WorkingDirectory $RootDir -DisplayName "npx tauri build --no-bundle"
        }

        if (Target-IncludesCli) {
            Write-Host "`n==> [3/3] CLI (cargo build --release --bin sugt-cli)..." -ForegroundColor Yellow
            Push-Location $TauriDir
            try {
                Invoke-Cargo @("build", "--release", "--bin", "sugt-cli")
            } finally {
                Pop-Location
            }
        }

        Write-Host "`n==> Build summary" -ForegroundColor Green
        foreach ($name in @("sugt.exe", "sugt-cli.exe")) {
            $p = Join-Path $TargetDir $name
            if (Test-Path $p) {
                $mb = [math]::Round((Get-Item $p).Length / 1MB, 2)
                Write-Host "    $name  ${mb} MB"
            }
        }
    }

    $sugtExe = Join-Path $TargetDir "sugt.exe"
    $cliExe = Join-Path $TargetDir "sugt-cli.exe"
    $iconSrc = Join-Path $TauriDir "icons\icon.ico"

    if (Target-IncludesGui) { Assert-FileExists -Path $sugtExe -Label "GUI (sugt.exe)" }
    if (Target-IncludesCli) { Assert-FileExists -Path $cliExe -Label "CLI (sugt-cli.exe)" }

    Write-Host "`n==> Assembling portable package..." -ForegroundColor Yellow

    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
    Clear-DirectoryContents -Path $outDir

    $files = @("README.txt", "VERSION.txt")
    if (Target-IncludesGui) {
        Copy-ReleaseFile -Source $sugtExe -Destination (Join-Path $outDir "SUGT.exe")
        $files += "SUGT.exe"
        if (Test-Path $iconSrc) {
            Copy-ReleaseFile -Source $iconSrc -Destination (Join-Path $outDir "SUGT.ico")
            $files += "SUGT.ico"
        }
    }
    if (Target-IncludesCli) {
        Copy-ReleaseFile -Source $cliExe -Destination (Join-Path $outDir "sugt-cli.exe")
        $files += "sugt-cli.exe"
    }

    $trialLine = if ($Variant -eq "trial") { "Internal trial build. Expires at: $expiresAt" } else { "Internal self-use build. No trial expiration." }
    $readme = @"
SUGT - su gateway Portable
==========================

Version: $version
Product: $Product
Variant: $Variant
Build ID: $buildId
$trialLine
Platform: Windows x64 portable

Runtime requirements:
- Windows 10 / 11 x64
- WebView2 Runtime for GUI
- No Rust, Node.js, or development tools required

Quick start:
1. Double-click SUGT.exe to open the GUI.
2. Configure model provider settings and start the gateway.
3. Use sugt-cli.exe for command-line gateway operations.

CLI examples:
  sugt-cli.exe status
  sugt-cli.exe serve
  sugt-cli.exe test
  sugt-cli.exe init --base-url <url> --api-key <key> --model <model>

Config directory:
  Documents\.sugt\config.toml
  Or set SUGT_CONFIG_DIR to a custom directory.

Trial state:
  Documents\.sugt\trial-state.json
  Runtime files are not written to the exe directory.
"@

    $versionTxt = @"
product=SUGT
product_line=$Product
version=$version
variant=$Variant
target=$Target
platform=windows-x64
build_id=$buildId
build_time=$(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
trial_enabled=$($Variant -eq "trial")
trial_expires_at=$expiresAt
"@

    [System.IO.File]::WriteAllText((Join-Path $outDir "README.txt"), $readme, $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $outDir "VERSION.txt"), $versionTxt, $utf8NoBom)

    $manifest = @{
        name = $packageName
        version = $version
        product_line = $Product
        variant = $Variant
        target = $Target
        platform = "windows-x64"
        package_type = "portable"
        files = $files
        build_id = $buildId
        trial_enabled = ($Variant -eq "trial")
        trial_expires_at = $expiresAt
        built_at = (Get-Date -Format "o")
    } | ConvertTo-Json -Depth 3

    [System.IO.File]::WriteAllText((Join-Path $outDir "manifest.json"), $manifest, $utf8NoBom)

    if ($Zip) {
        $zipPath = Join-Path $ReleaseRoot "$packageName.zip"
        if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
        Write-Host "`n==> Creating zip..." -ForegroundColor Yellow
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [System.IO.Compression.ZipFile]::CreateFromDirectory($outDir, $zipPath, [System.IO.Compression.CompressionLevel]::Optimal, $false)
        if (-not (Test-Path $zipPath)) {
            throw "Zip creation failed: $zipPath"
        }
        Write-Host "    Zip: $zipPath" -ForegroundColor Green
    }

    Write-Host "`n==> Done!" -ForegroundColor Green
    Write-Host "    Package: $outDir"
    Get-ChildItem $outDir | ForEach-Object {
        $sizeMB = [math]::Round($_.Length / 1MB, 2)
        Write-Host ("    {0,-16} {1,8} MB" -f $_.Name, $sizeMB)
    }

    if ($Test) {
        Write-Host "`n==> Post-build connectivity test..." -ForegroundColor Yellow
        $testScript = Join-Path $PSScriptRoot "test-connectivity.ps1"
        if (-not $env:SUGT_TEST_API_KEY) {
            Write-Warning "SUGT_TEST_API_KEY is not set. Skipping connectivity test."
        } else {
            & $testScript
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
    }
} finally {
    Clear-BuildMetadata
}
