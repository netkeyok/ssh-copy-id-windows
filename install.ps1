#Requires -Version 5.1
<#
.SYNOPSIS
    Installs ssh-copy-id for Windows from GitHub releases.

.DESCRIPTION
    Downloads the latest (or a specific) release, installs ssh-copy-id.exe into
    %LOCALAPPDATA%\Programs\ssh-copy-id and adds that folder to the user PATH.

.PARAMETER Version
    Release to install: "latest" (default) or a version such as 0.1.0 / v0.1.0.

.PARAMETER InstallDir
    Target directory. Defaults to %LOCALAPPDATA%\Programs\ssh-copy-id.

.PARAMETER NoPathUpdate
    Do not modify the user PATH.

.EXAMPLE
    irm https://raw.githubusercontent.com/netkeyok/ssh-copy-id-windows/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version 0.1.0
#>
[CmdletBinding()]
param(
    [string]$Version = 'latest',
    [string]$InstallDir,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repository = 'netkeyok/ssh-copy-id-windows'

if (-not $InstallDir) {
    $InstallDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\ssh-copy-id'
}

# Windows PowerShell 5.1 may still default to TLS 1.0.
try {
    $tls12 = [Net.SecurityProtocolType]::Tls12
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor $tls12
} catch {
    # Older .NET without Tls12: let the download try with system defaults.
}

if ($Version -eq 'latest') {
    $url = "https://github.com/$repository/releases/latest/download/ssh-copy-id.exe"
} else {
    $tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
    $url = "https://github.com/$repository/releases/download/$tag/ssh-copy-id.exe"
}

Write-Host "Downloading $url"
$tempFile = Join-Path ([IO.Path]::GetTempPath()) ("ssh-copy-id-" + [guid]::NewGuid().ToString('N') + ".exe")
try {
    Invoke-WebRequest -Uri $url -OutFile $tempFile -UseBasicParsing
} catch {
    throw "Download failed: $($_.Exception.Message)"
}

# Refuse to install something that is not a PE executable (e.g. an HTML 404 page).
$bytes = [IO.File]::ReadAllBytes($tempFile)
if ($bytes.Length -lt 100000 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    Remove-Item -LiteralPath $tempFile -Force -ErrorAction SilentlyContinue
    throw "Downloaded file is not a valid Windows executable. Check that release '$Version' exists."
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$exePath = Join-Path $InstallDir 'ssh-copy-id.exe'
Copy-Item -LiteralPath $tempFile -Destination $exePath -Force
Remove-Item -LiteralPath $tempFile -Force

Write-Host "Installed: $exePath"
& $exePath --version

function Add-ToUserPath {
    param([Parameter(Mandatory)][string]$Directory)

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }

    $entries = @($userPath.TrimEnd(';') -split ';' | Where-Object { $_ -ne '' })
    if ($entries -contains $Directory) {
        return $false
    }

    [Environment]::SetEnvironmentVariable('Path', (@($entries) + $Directory) -join ';', 'User')
    return $true
}

if (-not $NoPathUpdate) {
    if (Add-ToUserPath -Directory $InstallDir) {
        Write-Host "Added to user PATH: $InstallDir"

        # Tell Explorer and other already running shells about the new variable.
        try {
            if (-not ('Win32.EnvironmentBroadcast' -as [type])) {
                Add-Type -Namespace Win32 -Name EnvironmentBroadcast -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
            }
            $broadcastResult = [UIntPtr]::Zero
            [void][Win32.EnvironmentBroadcast]::SendMessageTimeout(
                [IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$broadcastResult)
        } catch {
            Write-Warning "Could not notify running shells about the PATH change."
        }
    } else {
        Write-Host "Already in user PATH: $InstallDir"
    }

    # Make the command usable in this very session too.
    if (($env:Path -split ';') -notcontains $InstallDir) {
        $env:Path = "$($env:Path.TrimEnd(';'));$InstallDir"
    }
}

Write-Host ''
Write-Host 'ssh-copy-id is ready. Try:' -ForegroundColor Green
Write-Host '  ssh-copy-id user@host'
Write-Host ''
Write-Host 'If the command is not found, open a new terminal window.'
