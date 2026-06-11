#Requires -Version 5.1
param(
    [string]$PayloadFile,
    [string]$Title,
    [string]$Milestone,
    [string[]]$NewFiles = @(),
    [object[]]$ModifiedFiles = @(),
    [string]$LogicSummary = "",
    [string]$TestResult = "",
    [string]$Emoji = "",
    [string]$AccessToken = $env:DINGTALK_ACCESS_TOKEN,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding $false
[Console]::InputEncoding  = $utf8
[Console]::OutputEncoding = $utf8
if ($PSVersionTable.PSVersion.Major -lt 6) { chcp 65001 | Out-Null }

$ScriptDir = $PSScriptRoot
$RootDir   = Split-Path $ScriptDir -Parent

function Read-Utf8([string]$Path) {
    return [System.IO.File]::ReadAllText((Resolve-Path $Path).Path, $utf8)
}

function Escape-JsonString([string]$Value) {
    if ($null -eq $Value) { return '""' }
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    foreach ($ch in $Value.ToCharArray()) {
        switch ($ch) {
            '"'  { [void]$sb.Append('\"') }
            '\'  { [void]$sb.Append('\\') }
            "`n" { [void]$sb.Append('\n') }
            "`r" { [void]$sb.Append('\r') }
            "`t" { [void]$sb.Append('\t') }
            default {
                $code = [int][char]$ch
                if ($code -lt 0x20) { [void]$sb.Append(('\u{0:x4}' -f $code)) }
                else { [void]$sb.Append($ch) }
            }
        }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
}

function Fmt([string]$Template, [hashtable]$Vars) {
    $r = $Template
    foreach ($k in $Vars.Keys) {
        $r = $r.Replace('{' + $k + '}', [string]$Vars[$k])
    }
    return $r
}

function Build-MarkdownBody($T, $Title, $Milestone, $NewFiles, $ModifiedFiles, $LogicSummary, $TestResult, $Emoji) {
    if (-not $Emoji) { $Emoji = $T.defaultEmoji }
    $now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

    $lines = @()
    $lines += Fmt $T.header @{ emoji = $Emoji; title = $Title }
    $lines += ""
    $lines += $T.reportLine
    $lines += ""
    $lines += $T.tableHeader
    $lines += $T.tableSep
    $lines += Fmt $T.rowTime      @{ value = $now }
    $lines += Fmt $T.rowProject   @{ value = $T.defaultProject }
    $lines += Fmt $T.rowMilestone @{ value = $Milestone }

    if ($NewFiles -and $NewFiles.Count -gt 0) {
        $fileList = ($NewFiles | ForEach-Object { '``' + $_ + '``' }) -join "<br/>"
        $lines += Fmt $T.rowNewFiles @{ value = $fileList }
    }

    if ($ModifiedFiles -and $ModifiedFiles.Count -gt 0) {
        $modList = ($ModifiedFiles | ForEach-Object {
            $item = $_
            if ($item -is [hashtable]) {
                $p = if ($item.Path) { $item.Path } else { $item.path }
                $c = if ($item.Change) { $item.Change } else { $item.change }
            } else {
                $p = $item.path; $c = $item.change
            }
            if ($p) { '``' + $p + '``' + $T.fileSep + $c } else { $c }
        }) -join "<br/>"
        $lines += Fmt $T.rowModified @{ value = $modList }
    }

    if ($LogicSummary) {
        $summary = ($LogicSummary -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ }) -join "<br/>"
        $lines += Fmt $T.rowLogic @{ value = $summary }
    }

    if ($TestResult) {
        $lines += Fmt $T.rowTest @{ value = $TestResult }
    }

    $lines += ""
    $lines += "---"
    $lines += ""
    $lines += $T.footer

    return @{
        Title = $T.titlePrefix + $Title
        Text  = ($lines -join "`n`n")
    }
}

# Load UTF-8 template (all Chinese strings live here)
$templatePath = Join-Path $ScriptDir "dingtalk-template.json"
if (-not (Test-Path $templatePath)) {
    Write-Error "Missing dingtalk-template.json"
    exit 1
}
$T = Read-Utf8 $templatePath | ConvertFrom-Json

# Load payload
if ($PayloadFile) {
    if (-not (Test-Path $PayloadFile)) {
        Write-Error "PayloadFile not found: $PayloadFile"
        exit 1
    }
    $payload = Read-Utf8 $PayloadFile | ConvertFrom-Json
    $Title         = $payload.title
    $Milestone     = $payload.milestone
    $NewFiles      = @($payload.newFiles)
    $LogicSummary  = $payload.logicSummary
    $TestResult    = $payload.testResult
    $Emoji         = $payload.emoji
    $ModifiedFiles = @()
    if ($payload.modifiedFiles) { $ModifiedFiles = @($payload.modifiedFiles) }
}

if (-not $Title -or -not $Milestone) {
    Write-Error "Required: -Title and -Milestone, or -PayloadFile"
    exit 1
}

# Load token
$envFile = Join-Path $RootDir ".env.dingtalk.local"
if (-not $AccessToken -and (Test-Path $envFile)) {
    [System.IO.File]::ReadAllLines((Resolve-Path $envFile).Path, $utf8) | ForEach-Object {
        if ($_ -match '^\s*DINGTALK_ACCESS_TOKEN\s*=\s*(.+)\s*$') {
            $AccessToken = $Matches[1].Trim().Trim('"').Trim("'")
        }
    }
}
if (-not $AccessToken) {
    Write-Error "DINGTALK_ACCESS_TOKEN not set"
    exit 1
}

$webhookUrl = "https://oapi.dingtalk.com/robot/send?access_token=$AccessToken"
$md = Build-MarkdownBody $T $Title $Milestone $NewFiles $ModifiedFiles $LogicSummary $TestResult $Emoji

$jsonBody = '{"msgtype":"markdown","markdown":{"title":'
$jsonBody += (Escape-JsonString $md.Title) + ',"text":'
$jsonBody += (Escape-JsonString $md.Text) + '}}'
$bodyBytes = $utf8.GetBytes($jsonBody)

if ($DryRun) {
    Write-Host "=== Dry Run ===" -ForegroundColor Cyan
    Write-Host $utf8.GetString($bodyBytes)
    exit 0
}

try {
    $response = Invoke-RestMethod -Uri $webhookUrl -Method Post `
        -ContentType "application/json; charset=utf-8" -Body $bodyBytes
    if ($response.errcode -eq 0) {
        Write-Host "OK: $Milestone" -ForegroundColor Green
    } else {
        Write-Warning "errcode=$($response.errcode) errmsg=$($response.errmsg)"
        exit 1
    }
} catch {
    Write-Error "Send failed: $_"
    exit 1
}
