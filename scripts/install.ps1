param(
    [string]$Version,
    [switch]$VerboseMode
)

$ErrorActionPreference = "Stop"

function Write-Log {
    param([string]$Message)

    if ($VerboseMode) {
        Write-Host "[VERBOSE] $Message"
    }
}

$repo = "a2-ai/ghqctoolkit"
$releaseApiUrl = if ([string]::IsNullOrWhiteSpace($Version)) {
    "https://api.github.com/repos/$repo/releases/latest"
} else {
    "https://api.github.com/repos/$repo/releases/tags/$Version"
}
$installDir = Join-Path $env:LOCALAPPDATA "Programs\ghqc"
$zipSuffix = if ([string]::IsNullOrWhiteSpace($Version)) { "latest" } else { $Version }
$zipPath = Join-Path $env:TEMP "ghqc_$zipSuffix.zip"

Write-Log "Fetching release metadata from $releaseApiUrl"
$release = Invoke-RestMethod -Uri $releaseApiUrl

if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
    $target = "aarch64-pc-windows-msvc"
} else {
    $target = "x86_64-pc-windows-msvc"
}

$assetPattern = "ghqc-*-{0}.zip" -f $target
Write-Log "Looking for asset matching $assetPattern"

$asset = $release.assets | Where-Object { $_.name -like $assetPattern } | Select-Object -First 1

if (-not $asset) {
    $availableAssets = ($release.assets | ForEach-Object { $_.name }) -join ", "
    throw "Could not find a release asset matching '$assetPattern'. Available assets: $availableAssets"
}

Write-Host "Downloading $($asset.name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

if (Test-Path $installDir) {
    Write-Log "Removing existing install directory at $installDir"
    Remove-Item -Recurse -Force $installDir
}

Write-Host "Installing to $installDir..."
New-Item -ItemType Directory -Path $installDir -Force | Out-Null
Expand-Archive -Path $zipPath -DestinationPath $installDir -Force
Remove-Item -Force $zipPath

$targetExeName = "ghqc-$target.exe"
$targetExePath = Join-Path $installDir $targetExeName
$exePath = Join-Path $installDir "ghqc.exe"

if ((Test-Path $targetExePath) -and -not (Test-Path $exePath)) {
    Write-Log "Renaming $targetExeName to ghqc.exe"
    Rename-Item -Path $targetExePath -NewName "ghqc.exe"
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @()

if (-not [string]::IsNullOrWhiteSpace($userPath)) {
    $pathEntries = $userPath.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries)
}

if ($pathEntries -notcontains $installDir) {
    $newUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
        $installDir
    } else {
        "$userPath;$installDir"
    }

    [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    Write-Host "Added $installDir to your user PATH."
} else {
    Write-Log "$installDir is already present in the user PATH"
}

if (-not (Test-Path $exePath)) {
    throw "Installation completed, but ghqc.exe was not found in $installDir"
}

Write-Host "ghqc installed successfully to $exePath"
Write-Host "Open a new PowerShell window, then run: ghqc --version"
