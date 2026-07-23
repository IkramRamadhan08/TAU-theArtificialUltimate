param(
    [switch]$NoDesktop,
    [switch]$NoStartMenu,
    [switch]$Help
)

$REPO = "IkramRamadhan08/TAU-theArtificialUltimate"
$VERSION = "latest"

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-OK($msg) { Write-Host "  OK: $msg" -ForegroundColor Green }
function Write-Info($msg) { Write-Host "  $msg" -ForegroundColor Gray }
function Write-Error($msg) { Write-Host "  ERROR: $msg" -ForegroundColor Red }

if ($Help) {
    Write-Host @"

TAU Editor Windows Installer

Usage:
  powershell -ExecutionPolicy Bypass -File install.ps1

Options:
  -NoDesktop     Skip creating desktop shortcut
  -NoStartMenu   Skip creating Start Menu shortcut
  -Help          Show this help

One-liner:
  powershell -c "& { $(Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/$REPO/main/editor/script/install.ps1' -UseBasicParsing).Content | Invoke-Expression }"

"@
    exit 0
}

Write-Step "TAU Editor Installer for Windows"

# ----- Detect architecture -----
$ARCH = $env:PROCESSOR_ARCHITECTURE
if ($ARCH -eq "AMD64") {
    $ASSET = "tau-x86_64-windows.zip"
} elseif ($ARCH -eq "ARM64") {
    Write-Info "ARM64 detected, attempting ARM64 build..."
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
    # If ARM64 build not found, fallback to x86_64
    if ($ARCH -eq "ARM64") {
        Write-Info "ARM64 build not available, falling back to x86_64..."
        Write-Info "Note: x86_64 build will run under emulation (reduced performance)."
        $ASSET = "tau-x86_64-windows.zip"
        $DOWNLOAD_URL = "https://github.com/$REPO/releases/$VERSION/download/$ASSET"
        try {
            Invoke-WebRequest -Uri $DOWNLOAD_URL -OutFile $TEMP_ZIP -UseBasicParsing -TimeoutSec 600
            Write-OK "Downloaded x86_64 fallback ($([math]::Round((Get-Item $TEMP_ZIP).Length / 1MB, 1)) MB)"
        } catch {
            Write-Error "Download failed: $_"
            exit 1
        }
    } else {
        Write-Error "Download failed: $_"
        exit 1
    }
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

$EXE = Get-ChildItem -Path $TEMP_DIR -Filter "tau-x86_64-windows.exe" -Recurse | Select-Object -First 1
if (-not $EXE) {
    # Fallback: find any .exe that is not the CLI
    $EXE = Get-ChildItem -Path $TEMP_DIR -Filter "*.exe" -Recurse | Where-Object { $_.Name -notlike "tau-cli-*" } | Select-Object -First 1
}
if (-not $EXE) {
    Write-Error "Could not find tau.exe in archive"
    Remove-Item $TEMP_ZIP -Force -ErrorAction SilentlyContinue
    Remove-Item $TEMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# Look for CLI binary (optional, improves terminal functionality)
$CLI_EXE = Get-ChildItem -Path $TEMP_DIR -Filter "tau-cli-*.exe" -Recurse | Select-Object -First 1

# ----- Install -----
Write-Step "Installing to $INSTALL_DIR..."
if (-not (Test-Path $INSTALL_DIR)) {
    New-Item -ItemType Directory -Path $INSTALL_DIR -Force | Out-Null
}

try {
    Copy-Item -Path $EXE.FullName -Destination $BINARY_PATH -Force
    Write-OK "Installed tau.exe ($([math]::Round((Get-Item $BINARY_PATH).Length / 1MB, 1)) MB)"
    if ($CLI_EXE) {
        $CLI_PATH = "$INSTALL_DIR\tau-cli.exe"
        Copy-Item -Path $CLI_EXE.FullName -Destination $CLI_PATH -Force
        Write-OK "Installed tau-cli.exe (CLI launcher)"
    }
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

# ----- Start Menu shortcut (default) -----
if (-not $NoStartMenu) {
    Write-Step "Creating Start Menu shortcut..."
    $WScriptShell = New-Object -ComObject WScript.Shell
    $START_MENU = [Environment]::GetFolderPath("Programs")
    $SHORTCUT_DIR = "$START_MENU\TAU"
    if (-not (Test-Path $SHORTCUT_DIR)) {
        New-Item -ItemType Directory -Path $SHORTCUT_DIR -Force | Out-Null
    }
    $Shortcut = $WScriptShell.CreateShortcut("$SHORTCUT_DIR\TAU.lnk")
    $Shortcut.TargetPath = $BINARY_PATH
    $Shortcut.WorkingDirectory = $INSTALL_DIR
    $Shortcut.Description = "TAU - The Artificial Ultimate AI Code Editor"
    $Shortcut.Save()
    Write-OK "Start Menu shortcut created"
}

# ----- Desktop shortcut (default) -----
if (-not $NoDesktop) {
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

# ----- Register URI handler -----
Write-Step "Registering tau:// URI handler..."
try {
    $REG_BASE = "HKCU:\Software\Classes\tau"
    if (-not (Test-Path $REG_BASE)) {
        New-Item -Path $REG_BASE -Force | Out-Null
    }
    Set-ItemProperty -Path $REG_BASE -Name "(Default)" -Value "URL:TAU Protocol"
    Set-ItemProperty -Path $REG_BASE -Name "URL Protocol" -Value ""
    Set-ItemProperty -Path $REG_BASE -Name "AppUserModelID" -Value "ai.tau.TAU"
    $ICON_PATH = "$BINARY_PATH,0"
    $CMD = "`"$BINARY_PATH`" `"%1`""
    New-Item -Path "$REG_BASE\DefaultIcon" -Force | Out-Null
    Set-ItemProperty -Path "$REG_BASE\DefaultIcon" -Name "(Default)" -Value $ICON_PATH
    New-Item -Path "$REG_BASE\shell\open\command" -Force | Out-Null
    Set-ItemProperty -Path "$REG_BASE\shell\open\command" -Name "(Default)" -Value $CMD
    Write-OK "tau:// URI handler registered"
} catch {
    Write-Error "Failed to register URI handler: $_"
}

# ----- Register file associations -----
Write-Step "Registering file associations..."
$EXTENSIONS = @(
    ".py", ".js", ".ts", ".tsx", ".jsx", ".rs", ".go", ".java",
    ".c", ".cpp", ".h", ".hpp", ".cs", ".rb", ".php", ".swift",
    ".kt", ".scala", ".html", ".css", ".scss", ".less", ".json",
    ".yaml", ".yml", ".toml", ".xml", ".md", ".txt", ".sh",
    ".bash", ".zsh", ".fish", ".ps1", ".bat", ".cmd", ".sql",
    ".r", ".lua", ".zig", ".nim", ".ex", ".exs", ".hs", ".ml",
    ".fs", ".vb", ".pl", ".pm", ".dart", ".vue", ".svelte"
)

$REG_VALUE = "TAU.Editor"

foreach ($ext in $EXTENSIONS) {
    try {
        $extKey = "HKCU:\Software\Classes\$ext"
        if (-not (Test-Path $extKey)) {
            New-Item -Path $extKey -Force | Out-Null
        }
        $openWithKey = "$extKey\OpenWithProgids"
        if (-not (Test-Path $openWithKey)) {
            New-Item -Path $openWithKey -Force | Out-Null
        }
        Set-ItemProperty -Path $openWithKey -Name "$REG_VALUE" -Value "" -Type String
    } catch {
        # Skip failed extensions silently
    }
}

# Create ProgID
try {
    $progIdKey = "HKCU:\Software\Classes\$REG_VALUE"
    if (-not (Test-Path $progIdKey)) {
        New-Item -Path $progIdKey -Force | Out-Null
    }
    Set-ItemProperty -Path $progIdKey -Name "(Default)" -Value "TAU Source File"
    Set-ItemProperty -Path $progIdKey -Name "AppUserModelID" -Value "ai.tau.TAU"
    $defaultIconKey = "$progIdKey\DefaultIcon"
    if (-not (Test-Path $defaultIconKey)) {
        New-Item -Path $defaultIconKey -Force | Out-Null
    }
    Set-ItemProperty -Path $defaultIconKey -Name "(Default)" -Value "`"$BINARY_PATH`",0"
    $shellOpenKey = "$progIdKey\shell\open"
    if (-not (Test-Path $shellOpenKey)) {
        New-Item -Path $shellOpenKey -Force | Out-Null
    }
    Set-ItemProperty -Path $shellOpenKey -Name "Icon" -Value "`"$BINARY_PATH`""
    $commandKey = "$progIdKey\shell\open\command"
    if (-not (Test-Path $commandKey)) {
        New-Item -Path $commandKey -Force | Out-Null
    }
    Set-ItemProperty -Path $commandKey -Name "(Default)" -Value "`"$BINARY_PATH`" `"%1`""
    Write-OK "File associations registered"
} catch {
    Write-Error "Failed to register file associations: $_"
}

# ----- Done -----
Write-Host ""
Write-Host "=== TAU installed successfully! ===" -ForegroundColor Green
Write-Host ""
Write-Host "  You can now:" -ForegroundColor White
Write-Host "    - Open TAU from Start Menu" -ForegroundColor White
Write-Host "    - Double-click TAU shortcut on Desktop" -ForegroundColor White
Write-Host "    - Run 'tau' from PowerShell or Command Prompt" -ForegroundColor White
Write-Host "    - Right-click any code file and select 'Open with TAU'" -ForegroundColor White
Write-Host ""
Write-Host "  To pin to taskbar: right-click TAU in Start Menu > Pin to taskbar" -ForegroundColor Gray
Write-Host ""
