#Requires -Version 5.1
<#
.SYNOPSIS
    TAU Editor Windows Uninstaller

.DESCRIPTION
    Removes TAU Editor from Windows, including binary, shortcuts, registry entries, and optionally config files.

.PARAMETER KeepConfig
    Keep configuration files (settings, keymaps, themes)

.PARAMETER KeepAgents
    Keep global agent skills

.PARAMETER Help
    Show this help message

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File uninstall.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File uninstall.ps1 -KeepConfig
#>

param(
    [switch]$KeepConfig,
    [switch]$KeepAgents,
    [switch]$Help
)

if ($Help) {
    Get-Help $MyInvocation.MyCommand.Path -Detailed
    exit 0
}

Write-Host "=== TAU Editor Uninstaller ===" -ForegroundColor Cyan
Write-Host ""

# ---------- Paths ----------
$INSTALL_DIR = "$env:LOCALAPPDATA\TAU"
$BINARY_PATH = "$INSTALL_DIR\tau.exe"
$CONFIG_DIR = "$env:APPDATA\TAU"
$DATA_DIR = "$env:LOCALAPPDATA\TAU"
$AGENTS_DIR = "$env:USERPROFILE\.agents"

# ---------- Remove binary ----------
Write-Host "Removing binary..." -ForegroundColor Yellow
if (Test-Path $BINARY_PATH) {
    Remove-Item -Path $BINARY_PATH -Force
    Write-Host "  Removed $BINARY_PATH" -ForegroundColor Green
}

# Remove install directory if empty
if (Test-Path $INSTALL_DIR) {
    $remaining = Get-ChildItem -Path $INSTALL_DIR -ErrorAction SilentlyContinue
    if (-not $remaining) {
        Remove-Item -Path $INSTALL_DIR -Force
        Write-Host "  Removed $INSTALL_DIR" -ForegroundColor Green
    }
}

# ---------- Remove Start Menu shortcut ----------
Write-Host "Removing Start Menu shortcut..." -ForegroundColor Yellow
$WScriptShell = New-Object -ComObject WScript.Shell
$START_MENU = [Environment]::GetFolderPath("Programs")
$SHORTCUT_DIR = "$START_MENU\TAU"

if (Test-Path $SHORTCUT_DIR) {
    Remove-Item -Path $SHORTCUT_DIR -Recurse -Force
    Write-Host "  Removed Start Menu shortcut" -ForegroundColor Green
}

# ---------- Remove Desktop shortcut ----------
Write-Host "Removing Desktop shortcut..." -ForegroundColor Yellow
$DESKTOP = [Environment]::GetFolderPath("Desktop")
$DESKTOP_SHORTCUT = "$DESKTOP\TAU.lnk"

if (Test-Path $DESKTOP_SHORTCUT) {
    Remove-Item -Path $DESKTOP_SHORTCUT -Force
    Write-Host "  Removed Desktop shortcut" -ForegroundColor Green
}

# ---------- Remove URI handler ----------
Write-Host "Removing URI handler..." -ForegroundColor Yellow
$REG_BASE = "HKCU:\Software\Classes\tau"
if (Test-Path $REG_BASE) {
    Remove-Item -Path $REG_BASE -Recurse -Force
    Write-Host "  Removed tau:// URI handler" -ForegroundColor Green
}

# ---------- Remove file associations ----------
Write-Host "Removing file associations..." -ForegroundColor Yellow
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
$removed = 0

foreach ($ext in $EXTENSIONS) {
    try {
        $extKey = "HKCU:\Software\Classes\$ext"
        if (Test-Path $extKey) {
            $openWithKey = "$extKey\OpenWithProgids"
            if (Test-Path $openWithKey) {
                Remove-ItemProperty -Path $openWithKey -Name $REG_VALUE -Force -ErrorAction SilentlyContinue
                $removed++
            }
        }
    } catch {
        # Skip silently
    }
}

# Remove ProgID
$progIdKey = "HKCU:\Software\Classes\$REG_VALUE"
if (Test-Path $progIdKey) {
    Remove-Item -Path $progIdKey -Recurse -Force
    $removed++
}

Write-Host "  Removed file associations ($removed entries)" -ForegroundColor Green

# ---------- Remove from PATH ----------
Write-Host "Removing from PATH..." -ForegroundColor Yellow
$PATH_TARGET = "User"
$CURRENT_PATH = [Environment]::GetEnvironmentVariable("Path", $PATH_TARGET)

if ($CURRENT_PATH -like "*$INSTALL_DIR*") {
    $NEW_PATH = ($CURRENT_PATH -split ";" | Where-Object { $_ -ne $INSTALL_DIR }) -join ";"
    try {
        [Environment]::SetEnvironmentVariable("Path", $NEW_PATH, $PATH_TARGET)
        Write-Host "  Removed from User PATH" -ForegroundColor Green
    } catch {
        Write-Host "  Warning: Failed to update PATH: $_" -ForegroundColor Yellow
    }
}

# Remove from current session
$env:Path = ($env:Path -split ";" | Where-Object { $_ -ne $INSTALL_DIR }) -join ";"

# ---------- Remove data directory ----------
Write-Host "Removing data directory..." -ForegroundColor Yellow
if (Test-Path $DATA_DIR) {
    Remove-Item -Path $DATA_DIR -Recurse -Force
    Write-Host "  Removed $DATA_DIR" -ForegroundColor Green
}

# ---------- Remove config directory ----------
Write-Host ""
if (-not $KeepConfig) {
    if (Test-Path $CONFIG_DIR) {
        Write-Host "Remove TAU configuration files in $CONFIG_DIR?" -ForegroundColor Yellow
        Write-Host "  This includes settings, keymaps, themes." -ForegroundColor Gray
        $confirm = Read-Host "  (y/N)"
        if ($confirm -eq "y" -or $confirm -eq "Y") {
            Remove-Item -Path $CONFIG_DIR -Recurse -Force
            Write-Host "  Removed $CONFIG_DIR" -ForegroundColor Green
        } else {
            Write-Host "  Kept $CONFIG_DIR" -ForegroundColor Gray
        }
    }
} else {
    Write-Host "  Keeping config directory ($CONFIG_DIR)" -ForegroundColor Gray
}

# ---------- Remove agent skills ----------
Write-Host ""
if (-not $KeepAgents) {
    if (Test-Path $AGENTS_DIR) {
        Write-Host "Remove global agent skills in $AGENTS_DIR?" -ForegroundColor Yellow
        Write-Host "  This includes custom skills you've created." -ForegroundColor Gray
        $confirm = Read-Host "  (y/N)"
        if ($confirm -eq "y" -or $confirm -eq "Y") {
            Remove-Item -Path $AGENTS_DIR -Recurse -Force
            Write-Host "  Removed $AGENTS_DIR" -ForegroundColor Green
        } else {
            Write-Host "  Kept $AGENTS_DIR" -ForegroundColor Gray
        }
    }
} else {
    Write-Host "  Keeping agent skills ($AGENTS_DIR)" -ForegroundColor Gray
}

# ---------- Remove Copilot config ----------
$COPILOT_DIR = "$env:LOCALAPPDATA\github-copilot"
if (Test-Path $COPILOT_DIR) {
    Write-Host ""
    Write-Host "Remove GitHub Copilot config in $COPILOT_DIR?" -ForegroundColor Yellow
    $confirm = Read-Host "  (y/N)"
    if ($confirm -eq "y" -or $confirm -eq "Y") {
        Remove-Item -Path $COPILOT_DIR -Recurse -Force
        Write-Host "  Removed $COPILOT_DIR" -ForegroundColor Green
    } else {
        Write-Host "  Kept $COPILOT_DIR" -ForegroundColor Gray
    }
}

# ---------- Clean up temp files ----------
Write-Host ""
Write-Host "Cleaning temp files..." -ForegroundColor Yellow
$tempPatterns = @("tau-*.sock", "tau-etw-*.sock")
foreach ($pattern in $tempPatterns) {
    $tempFiles = Get-ChildItem -Path $env:TEMP -Filter $pattern -ErrorAction SilentlyContinue
    foreach ($file in $tempFiles) {
        Remove-Item -Path $file.FullName -Force -ErrorAction SilentlyContinue
    }
}

# ---------- Summary ----------
Write-Host ""
Write-Host "=== TAU has been uninstalled ===" -ForegroundColor Green
Write-Host ""
Write-Host "Removed:" -ForegroundColor White
Write-Host "  - Binary: $BINARY_PATH" -ForegroundColor Gray
Write-Host "  - Start Menu shortcut" -ForegroundColor Gray
Write-Host "  - Desktop shortcut" -ForegroundColor Gray
Write-Host "  - URI handler (tau://)" -ForegroundColor Gray
Write-Host "  - File associations" -ForegroundColor Gray
Write-Host "  - PATH entry" -ForegroundColor Gray
Write-Host "  - Data directory: $DATA_DIR" -ForegroundColor Gray

if (Test-Path $CONFIG_DIR) {
    Write-Host ""
    Write-Host "Kept: $CONFIG_DIR (user config)" -ForegroundColor Yellow
}
if (Test-Path $AGENTS_DIR) {
    Write-Host "Kept: $AGENTS_DIR (agent skills)" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Note: Per-project .tau/ directories were NOT removed." -ForegroundColor Gray
Write-Host "      Delete them manually from each project if needed." -ForegroundColor Gray
Write-Host ""
Write-Host "Close and reopen your terminal for PATH changes to take full effect." -ForegroundColor Gray
