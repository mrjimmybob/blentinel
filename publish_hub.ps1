param(
    [switch]$Help
)

function Show-Help {
    Write-Host "Publish HUB Script" -ForegroundColor Cyan
    Write-Host "==================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\publish_hub.ps1"
    Write-Host ""
    Write-Host "Builds the hub in release mode and produces a zip in .\publish\hub"
}

if ($args -contains "--help") { $Help = $true }
if ($Help) { Show-Help; exit 0 }

$publishRoot = "publish\hub"
$hubOut = "$publishRoot\app"

New-Item -ItemType Directory -Force -Path $hubOut | Out-Null

Write-Host "Publishing HUB (native)..." -ForegroundColor Green

# -------------------------
# Build hub (native only)
# -------------------------
.\build_hub.ps1 -Release
if ($LASTEXITCODE -ne 0) { exit 1 }

# Find release dir
$pprofile = "release"
$targetDir = "target\$pprofile"

$hubExe = if ($IsWindows) { "hub.exe" } else { "hub" }

Copy-Item "$targetDir\$hubExe" "$hubOut\" -Force

# -------------------------
# Generate hub config file
# -------------------------
$configFile = "$hubOut\blentinel_hub.toml"
@"
# ---------------------------------------------------------------------------
# Blentinel Hub Configuration
# ---------------------------------------------------------------------------

[server]
# Address and port the hub listens on
host = "127.0.0.1"
port = 3000

# SQLite database file (relative to the working directory)
db_path = "blentinel.db"

# Path to the persistent X25519 private key used for ECDH with probes.
# Generated automatically on first run if it does not exist.
identity_key_path = "hub_identity.key"

# Seconds of silence before a probe is marked expired.
# Should be at least 2-3x the probe's reporting interval.
probe_timeout_secs = 120

# ---------------------------------------------------------------------------
# TLS/HTTPS Configuration (Optional)
# ---------------------------------------------------------------------------
# Uncomment to enable HTTPS. Certificate auto-generated on first run.
# Copy hub_tls_cert.pem to probe/hub_cert.pem for certificate pinning.
#
# [server.tls]
# enabled = false
# cert_path = "hub_tls_cert.pem"
# key_path = "hub_tls_key.pem"
# https_port = 3443  # Optional: run HTTP and HTTPS on different ports

# ---------------------------------------------------------------------------
# Authorized Probes
# ---------------------------------------------------------------------------
# Each [[probes]] entry registers a probe the hub will accept reports from.
# public_key is the hex-encoded Ed25519 public key printed by the probe on
# its very first run. A probe whose ID is not listed here will be rejected.
#
# Example:
#   [[probes]]
#   name       = "SERVER-1"
#   public_key = "PUT_PROBE_PUBLIC_KEY_HERE"

[[probes]]
name = "SERVER-1"
public_key = "PUT_PROBE_PUBLIC_KEY_HERE"
"@ | Out-File -Encoding UTF8 $configFile

# -------------------------
# Generate service files
# -------------------------

# systemd service
$systemdFile = "$hubOut\blentinel-hub.service"
@"
[Unit]
Description=Blentinel Hub
After=network.target

[Service]
Type=simple
ExecStart=/opt/blentinel/hub/hub
Restart=always
RestartSec=5
User=blentinel
WorkingDirectory=/opt/blentinel/hub

[Install]
WantedBy=multi-user.target
"@ | Out-File -Encoding UTF8 $systemdFile

# Windows installer
$winInstaller = "$hubOut\install_hub_service.ps1"
@"
`$serviceName = "BlentinelHub"
`$installDir = "C:\Blentinel\hub"
`$exeName = "hub.exe"
`$exePath = Join-Path `$installDir `$exeName

Write-Host "Installing Blentinel Hub service..." -ForegroundColor Green

New-Item -ItemType Directory -Force -Path `$installDir | Out-Null
Copy-Item ".\`$exeName" `$exePath -Force
Copy-Item ".\blentinel_hub.toml" `$installDir -Force

sc.exe create `$serviceName binPath= "`"`$exePath`"" start= auto
sc.exe description `$serviceName "Blentinel central monitoring hub"

Start-Service `$serviceName

Write-Host "Service installed and started." -ForegroundColor Cyan
"@ | Out-File -Encoding UTF8 $winInstaller

# Linux installer
$linuxInstaller = "$hubOut\install_hub_service.sh"
@"
sudo mkdir -p /opt/blentinel/hub
sudo cp * /opt/blentinel/hub
sudo cp blentinel-hub.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable blentinel-hub
sudo systemctl start blentinel-hub
"@ | Out-File -Encoding UTF8 $linuxInstaller

# -------------------------
# Zip output
# -------------------------
$zipPath = "publish\hub.zip"
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }

Compress-Archive -Path $publishRoot\* -DestinationPath $zipPath

Write-Host "`nPublish output:" -ForegroundColor Cyan
Write-Host "  $publishRoot"
Write-Host "  $zipPath"

Write-Host "`nPublish completed." -ForegroundColor Green
