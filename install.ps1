param(
    [switch]$Desktop = $false,
    [switch]$Help
)

$REPO = "IkramRamadhan08/TAU-theArtificialUltimate"
$VERSION = "latest"

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-OK($msg) { Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Info($msg) { Write-Host "  $msg" -ForegroundColor Gray }
function Write-Error($msg) { Write-Host "  ✗ $msg" -ForegroundColor Red }

if ($Help) {
    Write-Host @"

TAU Editor Windows Installer

Usage:
  powershell -ExecutionPolicy Bypass -File install.ps1

Options:
  -Desktop   Create desktop shortcut
  -Help      Show this help

One-liner:
  powershell -c "& { $(Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/$REPO/main/install.ps1' -UseBasicParsing).Content | Invoke-Expression }"

"@
    exit 0
}

Write-Step "TAU Editor Installer for Windows"

# ----- Detect architecture -----
$ARCH = $env:PROCESSOR_ARCHITECTURE
if ($ARCH -eq "AMD64") {
    $ASSET = "tau-x86_64-windows.zip"
} elseif ($ARCH -eq "ARM64") {
    $ASSET = "tau-aarch64-windows.zip"
} else {
    Write-Error "Unsupported architecture: $ARCH"
    exit 1
}

$DOWNLOAD_URL = "https://github.com/$REPO/releases/$VERSION/download/$ASSET"
$INSTALL_DIR = "$env:LOCALAPPDATA\TAU"
$BINARY_PATH = "$INSTALL_DIR\tau.exe"

Write-Step "Downloading $ASSET..."

# ----- Download -----
$TEMP_ZIP = "$env:TEMP\tau_install.zip"
try {
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $DOWNLOAD_URL -OutFile $TEMP_ZIP -UseBasicParsing -TimeoutSec 600
    Write-OK "Downloaded ($([math]::Round((Get-Item $TEMP_ZIP).Length / 1MB, 1)) MB)"
} catch {
    Write-Error "Download failed: $_"
    exit 1
}

# ----- Extract -----
Write-Step "Extracting..."
$TEMP_DIR = "$env:TEMP\tau_install"
if (Test-Path $TEMP_DIR) { Remove-Item -Recurse -Force $TEMP_DIR }
New-Item -ItemType Directory -Path $TEMP_DIR -Force | Out-Null

try {
    Expand-Archive -Path $TEMP_ZIP -DestinationPath $TEMP_DIR -Force
} catch {
    Write-Error "Extraction failed: $_"
    Remove-Item $TEMP_ZIP -Force -ErrorAction SilentlyContinue
    exit 1
}

$EXE = Get-ChildItem -Path $TEMP_DIR -Filter "*.exe" -Recurse | Select-Object -First 1
if (-not $EXE) {
    Write-Error "Could not find tau.exe in archive"
    Remove-Item $TEMP_ZIP -Force -ErrorAction SilentlyContinue
    Remove-Item $TEMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# ----- Install -----
Write-Step "Installing to $INSTALL_DIR..."
if (-not (Test-Path $INSTALL_DIR)) {
    New-Item -ItemType Directory -Path $INSTALL_DIR -Force | Out-Null
}

try {
    Copy-Item -Path $EXE.FullName -Destination $BINARY_PATH -Force
    Write-OK "Installed tau.exe ($([math]::Round((Get-Item $BINARY_PATH).Length / 1MB, 1)) MB)"
} catch {
    Write-Error "Failed to copy binary: $_"
    Remove-Item $TEMP_ZIP -Force -ErrorAction SilentlyContinue
    Remove-Item $TEMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# Cleanup
Remove-Item $TEMP_ZIP -Force -ErrorAction SilentlyContinue
Remove-Item $TEMP_DIR -Recurse -Force -ErrorAction SilentlyContinue

# ----- Add to PATH -----
Write-Step "Adding to PATH..."
$PATH_TARGET = "User"
$CURRENT_PATH = [Environment]::GetEnvironmentVariable("Path", $PATH_TARGET)
if ($CURRENT_PATH -notlike "*$INSTALL_DIR*") {
    try {
        [Environment]::SetEnvironmentVariable("Path", "$CURRENT_PATH;$INSTALL_DIR", $PATH_TARGET)
        Write-OK "Added to User PATH"
        Write-Info "You may need to restart your terminal for PATH changes to take effect."
    } catch {
        Write-Error "Failed to update PATH: $_"
    }
} else {
    Write-Info "Already in PATH"
}

# Also set for current session
$env:Path = "$env:Path;$INSTALL_DIR"

# ----- Desktop shortcut (optional) -----
if ($Desktop) {
    Write-Step "Creating desktop shortcut..."
    $WScriptShell = New-Object -ComObject WScript.Shell
    $DESKTOP = [Environment]::GetFolderPath("Desktop")
    $Shortcut = $WScriptShell.CreateShortcut("$DESKTOP\TAU.lnk")
    $Shortcut.TargetPath = $BINARY_PATH
    $Shortcut.WorkingDirectory = $INSTALL_DIR
    $Shortcut.Description = "TAU - The Artificial Ultimate AI Code Editor"
    $Shortcut.Save()
    Write-OK "Desktop shortcut created"
}

# ----- Done -----
Write-Host ""
Write-Host "=== TAU installed! ===" -ForegroundColor Green
Write-Host ""
Write-Host "  Run 'tau' from PowerShell or Command Prompt." -ForegroundColor White
Write-Host "  The terminal will close automatically and TAU will appear." -ForegroundColor Gray
Write-Host ""
Write-Host "  To pin to taskbar: right-click tau.exe in $INSTALL_DIR and select 'Pin to taskbar'" -ForegroundColor Gray
Write-Host ""
