param(
    [switch]$Release,
    [switch]$Help,
    [string]$Target,
    [switch]$Watch
)

function Show-Help {
    Write-Host "Leptos Build Script" -ForegroundColor Cyan
    Write-Host "====================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Usage: .\leptos-build.ps1 [options]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Release              Build in release mode (optimized)"
    Write-Host "  -Target <target>      Specify target (e.g., wasm32-unknown-unknown)"
    Write-Host "  -Watch                Watch for changes and rebuild"
    Write-Host "  -Help                 Show this help message"
    Write-Host ""
}

if ($args -contains "--help") {
    $Help = $true
}
if ($Target -and $Target.StartsWith("--")) {
    Write-Host "Invalid target: $Target" -ForegroundColor Red
    Write-Host "Did you mean: -Help ?" -ForegroundColor Yellow
    exit 1
}

if ($Help) {
    Show-Help
    exit 0
}

# Check if cargo-leptos is installed
try {
    cargo leptos --help *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-leptos not found"
    }
}
catch {
    Write-Host "cargo-leptos is not installed. Installing..." -ForegroundColor Yellow
    cargo install cargo-leptos
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to install cargo-leptos." -ForegroundColor Red
        exit 1
    }
}

# Build argument list (ARRAY, not string)
$buildArgs = @("leptos", "build")

if ($Release) {
    $buildArgs += "--release"
}

if ($Target -ne "") {
    $buildArgs += "--target"
    $buildArgs += $Target
}

if ($Watch) {
    $buildArgs += "--watch"
}

Write-Host "Building Leptos application..." -ForegroundColor Green
Write-Host "Command: cargo $($buildArgs -join ' ')" -ForegroundColor Cyan

$startTime = Get-Date
cargo @buildArgs
$buildResult = $LASTEXITCODE
$endTime = Get-Date
$duration = $endTime - $startTime

if ($buildResult -eq 0) {
    Write-Host "`nBuild completed successfully!" -ForegroundColor Green
    Write-Host "Duration: $($duration.ToString('mm\:ss'))" -ForegroundColor Cyan
    
    $outputDir = "target/site"
    if (Test-Path $outputDir) {
        Write-Host "`nOutput directory: $outputDir" -ForegroundColor Yellow
    }
}
else {
    Write-Host "`nBuild failed with exit code: $buildResult" -ForegroundColor Red
    exit $buildResult
}
