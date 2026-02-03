param(
    [string]$Target = "",
    [switch]$Help
)

function Show-Help {
    Write-Host "Publish PROBE Script" -ForegroundColor Cyan
    Write-Host "====================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\publish_probe.ps1 -Target <triple>"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\publish_probe.ps1 -Target x86_64-unknown-linux-gnu (linux)"
    Write-Host "  .\publish_probe.ps1 -Target x86_64-unknown-linux-musl (linux-musl)"
    Write-Host "  .\publish_probe.ps1 -Target aarch64-unknown-linux-gnu (raspberry pi)"
    Write-Host "  .\publish_probe.ps1 -Target x86_64-pc-windows-msvc (windows)"
}

if ($args -contains "--help") { $Help = $true }
if ($Help) { Show-Help; exit 0 }

# -------------------------
# Auto-detect target
# -------------------------
if (-not $Target -or $Target -eq "") {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture

    if ($IsWindows) {
        if ($arch -eq "X64") { $Target = "x86_64-pc-windows-msvc" }
        elseif ($arch -eq "Arm64") { $Target = "aarch64-pc-windows-msvc" }
        else { Write-Host "Unsupported Windows architecture: $arch" -ForegroundColor Red; exit 1 }
    }
    elseif ($IsLinux) {
        if ($arch -eq "X64") { $Target = "x86_64-unknown-linux-gnu" }
        elseif ($arch -eq "Arm64") { $Target = "aarch64-unknown-linux-gnu" }
        else { Write-Host "Unsupported Linux architecture: $arch" -ForegroundColor Red; exit 1 }
    }
    elseif ($IsMacOS) {
        if ($arch -eq "X64") { $Target = "x86_64-apple-darwin" }
        elseif ($arch -eq "Arm64") { $Target = "aarch64-apple-darwin" }
        else { Write-Host "Unsupported macOS architecture: $arch" -ForegroundColor Red; exit 1 }
    }
    else {
        Write-Host "Unsupported operating system" -ForegroundColor Red
        exit 1
    }

    Write-Host "Auto-detected target: $Target" -ForegroundColor Cyan
}

$arch = $Target
$publishRoot = "publish\probe\$arch"
$probeOut = "$publishRoot\app"

New-Item -ItemType Directory -Force -Path $probeOut | Out-Null

Write-Host "Publishing PROBE for $Target" -ForegroundColor Green

# -------------------------
# Build probe
# -------------------------
.\build_probe.ps1 -Release -Target $Target
if ($LASTEXITCODE -ne 0) { exit 1 }

$pprofile = "release"
$targetDir = "target\$Target\$pprofile"

$probeExe = if ($Target -like "*windows*") { "probe.exe" } else { "probe" }

Copy-Item "$targetDir\$probeExe" "$probeOut\" -Force

# -------------------------
# Copy TLS certificate if exists
# -------------------------
$hubCert = "probe\hub_cert.pem"
if (Test-Path $hubCert) {
    Copy-Item $hubCert "$probeOut\hub_cert.pem" -Force
    Write-Host "Included hub TLS certificate for HTTPS support" -ForegroundColor Cyan
} else {
    Write-Host "[WARN] No hub_cert.pem found. Probe will only support HTTP." -ForegroundColor Yellow
}

# Strip (Linux)
if (-not ($Target -like "*windows*")) {
    if (Get-Command strip -ErrorAction SilentlyContinue) {
        strip "$probeOut\$probeExe"
    }
}

# -------------------------
# Generate config template
# -------------------------
$configFile = "$probeOut\blentinel_probe.toml"

@"
# ==================================
# Blentinel Probe Configuration File
# ==================================
# 1. Set your company_id
# 2. Set your hub_url (http://HUB_IP:PORT)
# 3. Adjust interval if needed
# 4. Add or remove [[resources]] blocks

[agent]
company_id = "COMPANY_NAME"
hub_url = "http://HUB_ADDRESS:PORT"
interval = 60


# ---- Ping (ICMP) ----
[[resources]]
name = "Router"
type = "ping"
target = "192.168.1.1"


# ---- HTTP ----
[[resources]]
name = "Company Website"
type = "http"
target = "https://example.com"


# ---- TCP Port ----
[[resources]]
name = "Database Server"
type = "tcp"
target = "192.168.1.50:5432"
"@ | Out-File -Encoding UTF8 $configFile

# -------------------------
# Generate service files
# -------------------------

# systemd service
$systemdFile = "$probeOut\blentinel-probe.service"
@"
[Unit]
Description=Blentinel Probe
After=network.target

[Service]
Type=simple
ExecStart=/opt/blentinel/probe/probe
Restart=always
RestartSec=5
User=blentinel
WorkingDirectory=/opt/blentinel/probe

[Install]
WantedBy=multi-user.target
"@ | Out-File -Encoding UTF8 $systemdFile

# Windows installer
$winInstaller = "$probeOut\install_probe_service.ps1"
@"
`$serviceName = "BlentinelProbe"
`$installDir = "C:\Blentinel\probe"
`$exeName = "probe.exe"
`$exePath = Join-Path `$installDir `$exeName

Write-Host "Installing Blentinel Probe service..." -ForegroundColor Green

New-Item -ItemType Directory -Force -Path `$installDir | Out-Null
Copy-Item ".\`$exeName" `$exePath -Force

sc.exe create `$serviceName binPath= "`"`$exePath`"" start= auto
sc.exe description `$serviceName "Blentinel network monitoring probe"

Start-Service `$serviceName

Write-Host "Service installed and started." -ForegroundColor Cyan
"@ | Out-File -Encoding UTF8 $winInstaller

# -------------------------
# Zip output
# -------------------------
$zipPath = "publish\probe-$arch.zip"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }

Compress-Archive -Path $publishRoot\* -DestinationPath $zipPath

Write-Host "`nPublish output:" -ForegroundColor Cyan
Write-Host "  $publishRoot"
Write-Host "  $zipPath"

Write-Host "`nPublish completed." -ForegroundColor Green
