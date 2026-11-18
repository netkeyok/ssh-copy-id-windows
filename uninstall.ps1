#Requires -Version 5.1
<#
.SYNOPSIS
    Removes ssh-copy-id installed by install.ps1.

.PARAMETER InstallDir
    Directory to remove. Defaults to %LOCALAPPDATA%\Programs\ssh-copy-id.

.EXAMPLE
    irm https://raw.githubusercontent.com/netkeyok/ssh-copy-id-windows/main/uninstall.ps1 | iex
#>
[CmdletBinding()]
param(
    [string]$InstallDir
)

$ErrorActionPreference = 'Stop'

if (-not $InstallDir) {
    $InstallDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\ssh-copy-id'
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $entries = @($userPath.TrimEnd(';') -split ';' | Where-Object { $_ -ne '' })
    $filtered = @($entries | Where-Object { $_ -ne $InstallDir })
    if ($filtered.Count -ne $entries.Count) {
        [Environment]::SetEnvironmentVariable('Path', ($filtered -join ';'), 'User')
        Write-Host "Removed from user PATH: $InstallDir"
    }
}

if (Test-Path -LiteralPath $InstallDir) {
    Remove-Item -LiteralPath $InstallDir -Recurse -Force
    Write-Host "Removed: $InstallDir"
} else {
    Write-Host "Nothing to remove at $InstallDir"
}

Write-Host ''
Write-Host 'Uninstalled. Open a new terminal window to refresh PATH.'
