$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$AppHost = if ($env:LAUNDRYKU_APP_HOST) { $env:LAUNDRYKU_APP_HOST } else { "127.0.0.1" }
$DefaultAppPort = if ($env:LAUNDRYKU_APP_PORT) { $env:LAUNDRYKU_APP_PORT } else { "8000" }
$AdminEmail = if ($env:LAUNDRYKU_ADMIN_EMAIL) { $env:LAUNDRYKU_ADMIN_EMAIL } else { "test@example.com" }
$AdminPassword = if ($env:LAUNDRYKU_ADMIN_PASSWORD) { $env:LAUNDRYKU_ADMIN_PASSWORD } else { "password" }
$DatabaseName = if ($env:LAUNDRYKU_DB_DATABASE) { $env:LAUNDRYKU_DB_DATABASE } else { "laundryku" }
$DatabasePort = if ($env:LAUNDRYKU_DB_PORT) { $env:LAUNDRYKU_DB_PORT } else { "3306" }

Set-Location $RootDir

function Test-Command {
    param([string] $Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Read-Default {
    param(
        [string] $Prompt,
        [string] $Default
    )

    $value = Read-Host "$Prompt [$Default]"
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $Default
    }

    return $value
}

function Read-YesNo {
    param(
        [string] $Prompt,
        [bool] $DefaultYes = $true
    )

    $suffix = if ($DefaultYes) { "Y/n" } else { "y/N" }
    $value = Read-Host "$Prompt [$suffix]"
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $DefaultYes
    }

    return $value.Trim().ToLowerInvariant().StartsWith("y")
}

function Install-WithWinget {
    param(
        [string] $PackageId,
        [string] $Name
    )

    if (-not (Test-Command winget)) {
        throw "winget is not available. Install $Name manually, then rerun this script."
    }

    Write-Host "Installing $Name..."
    winget install --id $PackageId --exact --silent --accept-package-agreements --accept-source-agreements
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
}

function Get-XamppPath {
    $candidates = @(
        "C:\xampp",
        "D:\xampp",
        "$env:SystemDrive\xampp"
    ) | Select-Object -Unique

    foreach ($candidate in $candidates) {
        if (Test-Path (Join-Path $candidate "mysql\bin\mysql.exe")) {
            return $candidate
        }
    }

    if (Read-YesNo "XAMPP was not found. Install XAMPP 8.2 with winget now?" $true) {
        Install-WithWinget "ApacheFriends.Xampp.8.2" "XAMPP"
        foreach ($candidate in $candidates) {
            if (Test-Path (Join-Path $candidate "mysql\bin\mysql.exe")) {
                return $candidate
            }
        }
    }

    $manualPath = Read-Default "Enter your XAMPP folder path" "C:\xampp"
    if (-not (Test-Path (Join-Path $manualPath "mysql\bin\mysql.exe"))) {
        throw "XAMPP MySQL was not found at $manualPath."
    }

    return $manualPath
}

function Get-PhpVersion {
    param([string] $PhpExe)

    $versionLine = & $PhpExe -r "echo PHP_VERSION;"
    return [version] $versionLine
}

function Resolve-Php {
    param([string] $XamppPath)

    $xamppPhp = Join-Path $XamppPath "php\php.exe"
    if (Test-Path $xamppPhp) {
        $xamppVersion = Get-PhpVersion $xamppPhp
        if ($xamppVersion -ge [version] "8.3.0") {
            Write-Host "Using XAMPP PHP $xamppVersion."
            return $xamppPhp
        }

        Write-Host "XAMPP PHP is $xamppVersion. This project requires PHP 8.3 or newer."
    }

    if (Test-Command php) {
        $systemPhp = (Get-Command php).Source
        $systemVersion = Get-PhpVersion $systemPhp
        if ($systemVersion -ge [version] "8.3.0") {
            Write-Host "Using system PHP $systemVersion."
            return $systemPhp
        }
    }

    if (Read-YesNo "Install PHP 8.4 with winget for Laravel?" $true) {
        Install-WithWinget "PHP.PHP.8.4" "PHP 8.4"
        if (Test-Command php) {
            $phpExe = (Get-Command php).Source
            $phpVersion = Get-PhpVersion $phpExe
            if ($phpVersion -ge [version] "8.3.0") {
                return $phpExe
            }
        }
    }

    throw "PHP 8.3 or newer is required."
}

function Ensure-CommandDependency {
    param(
        [string] $Command,
        [string] $PackageId,
        [string] $Name
    )

    if (Test-Command $Command) {
        return
    }

    if (Read-YesNo "$Name is missing. Install it with winget?" $true) {
        Install-WithWinget $PackageId $Name
    }

    if (-not (Test-Command $Command)) {
        throw "$Name is still missing."
    }
}

function Start-XamppMysql {
    param([string] $XamppPath)

    $mysqlBin = Join-Path $XamppPath "mysql\bin"
    $mysqlAdminExe = Join-Path $mysqlBin "mysqladmin.exe"
    $mysqlStart = Join-Path $XamppPath "mysql_start.bat"

    & $mysqlAdminExe --protocol=tcp -h127.0.0.1 -P$DatabasePort -uroot ping --silent 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) {
        return
    }

    Write-Host "Starting XAMPP MySQL..."
    if (Test-Path $mysqlStart) {
        Start-Process -FilePath $mysqlStart -WindowStyle Minimized | Out-Null
    } else {
        $mysqldExe = Join-Path $mysqlBin "mysqld.exe"
        $defaultsFile = Join-Path $mysqlBin "my.ini"
        Start-Process -FilePath $mysqldExe -ArgumentList "--defaults-file=$defaultsFile", "--standalone" -WindowStyle Hidden | Out-Null
    }

    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        & $mysqlAdminExe --protocol=tcp -h127.0.0.1 -P$DatabasePort -uroot ping --silent 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) {
            return
        }
    }

    throw "XAMPP MySQL could not be started. Check XAMPP Control Panel, then rerun this script."
}

function Set-EnvValue {
    param(
        [string] $Key,
        [string] $Value
    )

    $path = Join-Path $RootDir ".env"
    $line = "$Key=$Value"

    if (Test-Path $path) {
        $content = Get-Content $path
        $matched = $false
        $content = $content | ForEach-Object {
            if ($_ -match "^$([regex]::Escape($Key))=") {
                $matched = $true
                $line
            } else {
                $_
            }
        }

        if (-not $matched) {
            $content += $line
        }

        Set-Content -Path $path -Value $content -Encoding UTF8
    } else {
        Set-Content -Path $path -Value $line -Encoding UTF8
    }
}

Write-Host ""
Write-Host "Laundryku Windows installer"
Write-Host "This script uses XAMPP MySQL and starts Laravel with php artisan serve."
Write-Host ""

$XamppPath = Read-Default "XAMPP folder path" (Get-XamppPath)
$AppPort = Read-Default "Laravel app port" $DefaultAppPort

$MysqlBin = Join-Path $XamppPath "mysql\bin"
$MysqlExe = Join-Path $MysqlBin "mysql.exe"
$PhpExe = Resolve-Php $XamppPath
$PhpDir = Split-Path -Parent $PhpExe
$env:Path = "$PhpDir;$MysqlBin;$env:Path"

if (-not ((& $PhpExe -m) | Select-String -Pattern "^pdo_mysql$" -Quiet)) {
    throw "The selected PHP does not have pdo_mysql enabled. Enable pdo_mysql in php.ini, then rerun this script."
}

Ensure-CommandDependency "composer" "Composer.Composer" "Composer"
Ensure-CommandDependency "node" "OpenJS.NodeJS.LTS" "Node.js LTS"
Ensure-CommandDependency "npm" "OpenJS.NodeJS.LTS" "npm"

Start-XamppMysql $XamppPath

& $MysqlExe --protocol=tcp -h127.0.0.1 -P$DatabasePort -uroot -e "CREATE DATABASE IF NOT EXISTS ``$DatabaseName`` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"

if (-not (Test-Path ".env")) {
    Copy-Item ".env.example" ".env"
}

Set-EnvValue "APP_ENV" "local"
Set-EnvValue "APP_DEBUG" "true"
Set-EnvValue "APP_URL" "http://${AppHost}:${AppPort}"
Set-EnvValue "DB_CONNECTION" "mysql"
Set-EnvValue "DB_HOST" "127.0.0.1"
Set-EnvValue "DB_PORT" $DatabasePort
Set-EnvValue "DB_DATABASE" $DatabaseName
Set-EnvValue "DB_USERNAME" "root"
Set-EnvValue "DB_PASSWORD" ""
Set-EnvValue "SESSION_DRIVER" "file"
Set-EnvValue "QUEUE_CONNECTION" "sync"
Set-EnvValue "CACHE_STORE" "file"
Set-EnvValue "LAUNDRYKU_ADMIN_EMAIL" $AdminEmail
Set-EnvValue "LAUNDRYKU_ADMIN_PASSWORD" $AdminPassword

composer install --no-interaction --prefer-dist --optimize-autoloader
npm ci

$envContent = Get-Content ".env" -Raw
if ($envContent -notmatch "APP_KEY=base64:") {
    & $PhpExe artisan key:generate --force
}

& $PhpExe artisan config:clear
& $PhpExe artisan migrate --force
& $PhpExe artisan db:seed --force
npm run build

Write-Host ""
Write-Host "Laundryku is ready."
Write-Host "URL: http://${AppHost}:${AppPort}/admin"
Write-Host "Email: $AdminEmail"
Write-Host "Password: $AdminPassword"
Write-Host ""
& $PhpExe artisan serve --host=$AppHost --port=$AppPort
