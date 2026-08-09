#Requires -Version 5.1
<#
.SYNOPSIS
  Export an open-core public source tree from the full private workspace.

.DESCRIPTION
  Copies allowlisted paths into dist-public/sugt by default and refuses known
  proprietary paths (skill console / store, internal PRDs, secrets).
  The exported tree is for review and GitHub sync. Until compile-time store
  gating is finished, the tree may not build without additional edits.
#>
param(
  [string]$OutDir = "",
  [switch]$VerifyOnly,
  [switch]$Clean
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $OutDir) {
  $OutDir = Join-Path $Root "dist-public\sugt"
}

$DenyDirNames = @(
  "node_modules", "dist", "dist-public", "dist-release", "release",
  "target", ".vite", ".MemoryForAI", ".qwen", "baks", ".idea", ".vscode",
  ".secrets", ".git"
)

$DenyPathPrefixes = @(
  "src\store",
  "src/store",
  "src-tauri\src\store",
  "src-tauri/src/store"
)

$DenyFiles = @(
  "src\SkillsPanel.tsx",
  "src/SkillsPanel.tsx",
  "运行调试.md",
  "debug.md",
  "docs\PRD-sugt-v0.2.5.md",
  "docs\PRD-store-v0.2.md",
  "docs\功能点文档-v0.2.5.md",
  "docs\技术实现文档-v0.2.5.md",
  "docs\public-manifest.md",
  "docs\images\dev_wallpaper.jpg",
  ".env.dingtalk.local",
  ".env.dingtalk.local.example"
)

$DenyNamePatterns = @(
  "^\.notify-.*\.json$",
  "^\.env(\..+)?$",
  "^\.github$",  # file named .github (token footgun), not the directory
  "^PUBLIC_EXPORT\.txt$"
)

$AllowRootFiles = @(
  "LICENSE", "NOTICE", "README.md", "README.en.md", "package.json", "package-lock.json",
  "index.html", "tsconfig.json", "tsconfig.node.json", "vite.config.ts", ".gitignore"
)

$AllowDocFiles = @(
  "docs\open-source.md",
  "docs\BRANCHING.md",
  "docs\provider-vendors.md",
  "docs\build.md",
  "docs\images\README.md",
  "docs\images\.gitkeep"
)

function Normalize-Rel([string]$p) {
  return ($p -replace "/", "\").TrimStart(".\")
}

function Is-Denied([string]$rel) {
  $n = Normalize-Rel $rel
  foreach ($prefix in $DenyPathPrefixes) {
    $np = Normalize-Rel $prefix
    if ($n -eq $np -or $n.StartsWith($np + "\")) { return $true }
  }
  foreach ($f in $DenyFiles) {
    if ($n -eq (Normalize-Rel $f)) { return $true }
  }
  $leaf = Split-Path $n -Leaf
  foreach ($pat in $DenyNamePatterns) {
    if ($leaf -match $pat) { return $true }
  }
  return $false
}

function Assert-NoLeak([string]$base) {
  $leaks = @()
  $checks = @(
    "src\store",
    "src-tauri\src\store",
    "src\SkillsPanel.tsx",
    ".secrets",
    "docs\PRD-store-v0.2.md",
    "docs\PRD-sugt-v0.2.5.md",
    "docs\public-manifest.md",
    "docs\images\dev_wallpaper.jpg",
    "scripts\export-public-tree.ps1",
    "scripts\package-release.ps1",
    "scripts\package-release-all.ps1",
    "scripts\package-release-wizard.ps1",
    "scripts\release-settings.json",
    "scripts\RELEASE.md",
    "scripts\dingtalk-template.json",
    "scripts\dingtalk-payload.example.json",
    "PUBLIC_EXPORT.txt"
  )
  foreach ($c in $checks) {
    $p = Join-Path $base $c
    if (Test-Path $p) { $leaks += $c }
  }
  $notify = Get-ChildItem -Path (Join-Path $base "scripts") -Filter ".notify-*.json" -ErrorAction SilentlyContinue
  if ($notify) { $leaks += ($notify | ForEach-Object { "scripts\$($_.Name)" }) }
  if ($leaks.Count -gt 0) {
    throw ("Public tree leak detected: " + ($leaks -join ", "))
  }
}

Write-Host "==> SUGT public tree export"
Write-Host "    Root : $Root"
Write-Host "    Out  : $OutDir"

if ($VerifyOnly) {
  Write-Host "==> VerifyOnly: scanning private tree for deny-path presence (informational)"
  foreach ($p in $DenyPathPrefixes + $DenyFiles) {
    $full = Join-Path $Root (Normalize-Rel $p)
    if (Test-Path $full) {
      Write-Host ("    deny-present: " + (Normalize-Rel $p))
    }
  }
  Write-Host "==> VerifyOnly done (no files copied)"
  exit 0
}

if ($Clean -and (Test-Path $OutDir)) {
  Remove-Item -Recurse -Force $OutDir
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$copied = 0
$skipped = 0

function Copy-AllowedFile([string]$rel) {
  $script:n = Normalize-Rel $rel
  if (Is-Denied $n) {
    $script:skipped++
    return
  }
  $src = Join-Path $Root $n
  if (-not (Test-Path $src)) { return }
  $dst = Join-Path $OutDir $n
  $dstDir = Split-Path $dst -Parent
  if (-not (Test-Path $dstDir)) {
    New-Item -ItemType Directory -Force -Path $dstDir | Out-Null
  }
  Copy-Item -Force $src $dst
  $script:copied++
}

foreach ($f in $AllowRootFiles) { Copy-AllowedFile $f }
foreach ($f in $AllowDocFiles) { Copy-AllowedFile $f }

# README screenshots (png/webp/jpg under docs/images)
$imagesDir = Join-Path $Root "docs\images"
if (Test-Path $imagesDir) {
  Get-ChildItem -Path $imagesDir -File | Where-Object {
    $_.Extension -match '^\.(png|jpe?g|webp|gif)$'
  } | ForEach-Object {
    Copy-AllowedFile ("docs\images\" + $_.Name)
  }
}

function Rel-FromRoot([string]$fullName) {
  $prefix = $Root.TrimEnd("\") + "\"
  if ($fullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    return Normalize-Rel $fullName.Substring($prefix.Length)
  }
  return Normalize-Rel ($fullName.Substring($Root.Length).TrimStart("\", "/"))
}

function Copy-TreeFiltered([string]$relativeRoot) {
  $abs = Join-Path $Root $relativeRoot
  if (-not (Test-Path $abs)) { return }
  Get-ChildItem -Path $abs -Recurse -File -Force | ForEach-Object {
    $n = Rel-FromRoot $_.FullName
    foreach ($denyName in $DenyDirNames) {
      if ($n -match ("\\" + [regex]::Escape($denyName) + "\\")) { return }
    }
    if (Is-Denied $n) { $script:skipped++; return }
    Copy-AllowedFile $n
  }
}

# Frontend / backend / icons / capabilities — skip huge build dirs via DenyDirNames
Copy-TreeFiltered "src"
Copy-TreeFiltered "src-tauri\capabilities"
Copy-TreeFiltered "src-tauri\permissions"
Copy-TreeFiltered "src-tauri\icons"
Copy-AllowedFile "src-tauri\Cargo.toml"
Copy-AllowedFile "src-tauri\Cargo.lock"
Copy-AllowedFile "src-tauri\tauri.conf.json"
Copy-AllowedFile "src-tauri\build.rs"
Copy-TreeFiltered "src-tauri\src"

# Packaging / notify / export scripts stay in the private full tree only.
# No scripts/ directory is published to the open-core remote.

# Workflows (explicit copy; bypass deny helper so `.github` path is not mishandled)
$workflowFiles = @(
  ".github\workflows\ci.yml",
  ".github\workflows\release-feature.yml"
)
foreach ($wfRel in $workflowFiles) {
  $srcWf = Join-Path $Root $wfRel
  if (-not (Test-Path $srcWf)) {
    Write-Host ("    workflow-missing: " + $wfRel)
    continue
  }
  $dstWf = Join-Path $OutDir $wfRel
  $dstWfDir = Split-Path $dstWf -Parent
  New-Item -ItemType Directory -Force -Path $dstWfDir | Out-Null
  Copy-Item -Force $srcWf $dstWf
  $copied++
  Write-Host ("    workflow: " + $wfRel)
}

Assert-NoLeak $OutDir

$stampDir = Join-Path $Root "dist-public"
if (-not (Test-Path $stampDir)) {
  New-Item -ItemType Directory -Force -Path $stampDir | Out-Null
}
$stamp = Join-Path $stampDir "PUBLIC_EXPORT.txt"
@(
  "SUGT public tree export",
  ("generated_at=" + (Get-Date -Format "yyyy-MM-ddTHH:mm:ssK")),
  ("source_root=" + $Root),
  ("output=" + $OutDir),
  ("copied=" + $copied),
  ("skipped_denied=" + $skipped),
  "note=Marker kept outside exported tree. Skill console and release tooling excluded."
) | Set-Content -Path $stamp -Encoding UTF8

Write-Host "==> Done"
Write-Host "    copied=$copied skipped_denied=$skipped"
Write-Host "    output=$OutDir"
Write-Host "    marker=$stamp"
