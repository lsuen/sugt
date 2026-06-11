#Requires -Version 5.1
<#
.SYNOPSIS
  Test ModelScope upstream + SUGT gateway connectivity.

.EXAMPLE
  $env:SUGT_TEST_API_KEY = "ms-..."
  .\scripts\test-connectivity.ps1
#>
param(
    [string]$ApiKey = $env:SUGT_TEST_API_KEY,
    [string]$Model = "deepseek-ai/DeepSeek-V4-Flash",
    [string]$OpenAiBase = "https://api-inference.modelscope.cn/v1",
    [string]$AnthropicBase = "https://api-inference.modelscope.cn",
    [int]$GatewayPort = 18787,
    [switch]$SkipGateway,
    [switch]$SkipUpstream
)

$ErrorActionPreference = "Stop"

function Write-Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "  OK  $msg" -ForegroundColor Green }
function Write-Fail($msg) { Write-Host "  FAIL $msg" -ForegroundColor Red }

if (-not $ApiKey) {
    Write-Error "Set -ApiKey or env SUGT_TEST_API_KEY"
}

$RootDir = Split-Path $PSScriptRoot -Parent
$TauriDir = Join-Path $RootDir "src-tauri"
$CliExe = Join-Path $TauriDir "target\release\sugt-cli.exe"

$failed = 0

if (-not $SkipUpstream) {
    Write-Step "Upstream OpenAI chat/completions"
    $openAiBody = @{
        model = $Model
        messages = @(@{ role = "user"; content = "ping" })
        max_tokens = 1
        stream = $false
    } | ConvertTo-Json -Depth 5

    try {
        $resp = Invoke-WebRequest -Uri "$OpenAiBase/chat/completions" -Method POST `
            -Headers @{ Authorization = "Bearer $ApiKey"; "Content-Type" = "application/json" } `
            -Body $openAiBody -TimeoutSec 30 -UseBasicParsing
        if ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 300) {
            Write-Ok "OpenAI $OpenAiBase/chat/completions HTTP $($resp.StatusCode)"
        } else {
            Write-Fail "OpenAI HTTP $($resp.StatusCode): $($resp.Content)"
            $failed++
        }
    } catch {
        Write-Fail "OpenAI: $($_.Exception.Message)"
        $failed++
    }

    Write-Step "Upstream Anthropic /v1/messages"
    $anthropicBody = @{
        model = $Model
        max_tokens = 1
        messages = @(@{ role = "user"; content = "ping" })
    } | ConvertTo-Json -Depth 5

    try {
        $resp = Invoke-WebRequest -Uri "$AnthropicBase/v1/messages" -Method POST `
            -Headers @{
                "x-api-key" = $ApiKey
                "anthropic-version" = "2023-06-01"
                "Content-Type" = "application/json"
            } `
            -Body $anthropicBody -TimeoutSec 30 -UseBasicParsing
        if ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 300) {
            Write-Ok "Anthropic $AnthropicBase/v1/messages HTTP $($resp.StatusCode)"
        } else {
            Write-Fail "Anthropic HTTP $($resp.StatusCode): $($resp.Content)"
            $failed++
        }
    } catch {
        $status = $null
        if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
        if ($status -eq 404) {
            Write-Host "  WARN Anthropic 404 (ModelScope may be OpenAI-only)" -ForegroundColor Yellow
        } else {
            Write-Fail "Anthropic: $($_.Exception.Message)"
            $failed++
        }
    }
}

if (-not $SkipGateway) {
    if (-not (Test-Path $CliExe)) {
        Write-Error "Missing $CliExe - run npm run package:release first"
    }

    $testConfigDir = Join-Path $env:TEMP "sugt-connectivity-test"
    if (Test-Path $testConfigDir) { Remove-Item $testConfigDir -Recurse -Force }
    New-Item -ItemType Directory -Path $testConfigDir -Force | Out-Null

    $env:SUGT_CONFIG_DIR = $testConfigDir

    Write-Step "SUGT CLI init (temp config: $testConfigDir)"
    & $CliExe init --base-url $OpenAiBase --api-key $ApiKey --model $Model
    if ($LASTEXITCODE -ne 0) { throw "sugt-cli init failed" }
    Write-Ok "config initialized"

    Write-Step "SUGT CLI test (same logic as GUI test button)"
    $testOut = & $CliExe test 2>&1
    $testOut | ForEach-Object { Write-Host "  $_" }
    if ($testOut -match "Unavailable") {
        Write-Fail "sugt-cli test reported Unavailable"
        $failed++
    } else {
        Write-Ok "sugt-cli test"
    }

    Write-Step "SUGT gateway local proxy"
    $serveJob = Start-Job -ScriptBlock {
        param($exe, $cfgDir, $port)
        $env:SUGT_CONFIG_DIR = $cfgDir
        & $exe serve --host 127.0.0.1 --port $port
    } -ArgumentList $CliExe, $testConfigDir, $GatewayPort

    Start-Sleep -Seconds 2
    try {
        $localOpenAi = @{
            model = $Model
            messages = @(@{ role = "user"; content = "ping" })
            max_tokens = 1
            stream = $false
        } | ConvertTo-Json -Depth 5

        $chatResp = Invoke-WebRequest -Uri "http://127.0.0.1:$GatewayPort/v1/chat/completions" -Method POST `
            -Headers @{ "Content-Type" = "application/json" } `
            -Body $localOpenAi -TimeoutSec 30 -UseBasicParsing
        Write-Ok "local /v1/chat/completions HTTP $($chatResp.StatusCode)"

        $localAnthropic = @{
            model = $Model
            max_tokens = 1
            messages = @(@{ role = "user"; content = "ping" })
        } | ConvertTo-Json -Depth 5

        try {
            $msgResp = Invoke-WebRequest -Uri "http://127.0.0.1:$GatewayPort/v1/messages" -Method POST `
                -Headers @{
                    "x-api-key" = "local-test"
                    "anthropic-version" = "2023-06-01"
                    "Content-Type" = "application/json"
                } `
                -Body $localAnthropic -TimeoutSec 30 -UseBasicParsing
            Write-Ok "local /v1/messages HTTP $($msgResp.StatusCode)"
        } catch {
            Write-Fail "local /v1/messages: $($_.Exception.Message)"
            $failed++
        }
    } catch {
        Write-Fail "local gateway: $($_.Exception.Message)"
        $failed++
    } finally {
        Stop-Job $serveJob -ErrorAction SilentlyContinue
        Remove-Job $serveJob -Force -ErrorAction SilentlyContinue
        Remove-Item $testConfigDir -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item Env:SUGT_CONFIG_DIR -ErrorAction SilentlyContinue
    }
}

Write-Host ""
if ($failed -gt 0) {
    Write-Fail "$failed check(s) failed"
    exit 1
}
Write-Ok "all connectivity checks passed"
exit 0
