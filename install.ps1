param(
    [switch]$Update
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Set YTGET_REPO=owner/repository or replace the placeholder before publishing.
$Repo = $env:YTGET_REPO
if ([string]::IsNullOrWhiteSpace($Repo)) { $Repo = 'YOUR_GITHUB_USERNAME/ytget-rs' }
if ($Repo -like '*YOUR_GITHUB_USERNAME*' -or $Repo -notmatch '^[^/]+/[^/]+$') {
    throw 'Set $env:YTGET_REPO to owner/repository or replace YOUR_GITHUB_USERNAME/ytget-rs in install.ps1 and update.ps1 before publishing.'
}

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
if ($arch -ne 'X64') {
    throw "This Windows release currently supports x64 only. Detected architecture: $arch."
}
$Target = 'x86_64-pc-windows-msvc'
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\ytget-rs\bin'
$AppAsset = "ytget-rs-$Target.zip"
$YtDlpAsset = 'yt-dlp.exe'
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ytget-rs-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $TempDir, $InstallDir | Out-Null

function Download-File([string]$Url, [string]$Destination) {
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing -Headers @{ 'User-Agent' = 'ytget-rs-installer' }
}
function Get-ManifestHash([string]$ManifestPath, [string]$AssetName) {
    foreach ($line in Get-Content -LiteralPath $ManifestPath) {
        if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$') {
            $name = $Matches[2].Trim()
            if ($name -eq $AssetName) { return $Matches[1].ToLowerInvariant() }
        }
    }
    return $null
}
function Verify-Hash([string]$FilePath, [string]$AssetName, [string]$ManifestPath) {
    $expected = Get-ManifestHash $ManifestPath $AssetName
    if (-not $expected) { throw "No SHA-256 checksum for $AssetName; refusing to install it." }
    $actual = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "SHA-256 verification failed for $AssetName; file was not installed." }
}
function Install-FFmpeg {
    Write-Host 'Checking FFmpeg...' -ForegroundColor Cyan
    # FFmpeg.org links to these Windows builds from its download page.
    $url = 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip'
    $zipPath = Join-Path $TempDir 'ffmpeg-release-essentials.zip'
    $sumPath = Join-Path $TempDir 'ffmpeg-release-essentials.zip.sha256'
    Download-File $url $zipPath
    try {
        Download-File "$($url).sha256" $sumPath
        $sumText = Get-Content -Raw -LiteralPath $sumPath
        if ($sumText -match '([0-9a-fA-F]{64})') {
            $expected = $Matches[1].ToLowerInvariant()
        } else {
            throw 'The FFmpeg checksum file did not contain a SHA-256 value.'
        }
        $actual = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw 'FFmpeg ZIP SHA-256 verification failed; file was not installed.' }
    }
    catch {
        Remove-Item -Force -ErrorAction SilentlyContinue $zipPath
        throw "Could not verify the FFmpeg archive from its published SHA-256 checksum. $($_.Exception.Message)"
    }

    $extractDir = Join-Path $TempDir 'ffmpeg-extracted'
    Expand-Archive -LiteralPath $zipPath -DestinationPath $extractDir -Force
    $ffmpeg = Get-ChildItem -LiteralPath $extractDir -Filter 'ffmpeg.exe' -File -Recurse | Select-Object -First 1
    $ffprobe = Get-ChildItem -LiteralPath $extractDir -Filter 'ffprobe.exe' -File -Recurse | Select-Object -First 1
    if (-not $ffmpeg -or -not $ffprobe) { throw 'The FFmpeg archive did not contain ffmpeg.exe and ffprobe.exe.' }

    # Replace FFmpeg executables and any bundled DLLs as a unit, keeping the app and yt-dlp.
    Get-ChildItem -LiteralPath $InstallDir -File | Where-Object {
        $_.Name -in @('ffmpeg.exe', 'ffprobe.exe', 'ffplay.exe') -or $_.Extension -eq '.dll'
    } | Remove-Item -Force -ErrorAction SilentlyContinue
    $ffmpegBin = $ffmpeg.Directory.FullName
    Get-ChildItem -LiteralPath $ffmpegBin -File | Where-Object {
        $_.Name -in @('ffmpeg.exe', 'ffprobe.exe', 'ffplay.exe') -or $_.Extension -eq '.dll'
    } | Copy-Item -Destination $InstallDir -Force
    & (Join-Path $InstallDir 'ffmpeg.exe') -version | Select-Object -First 1
}

try {
    if ($Update) { Write-Host 'Updating ytget-rs and all managed dependencies...' -ForegroundColor Cyan }
    else { Write-Host 'Installing ytget-rs and its dependencies...' -ForegroundColor Cyan }

    Install-FFmpeg

    Write-Host 'Downloading the official yt-dlp release...'
    $ytDlpPath = Join-Path $TempDir $YtDlpAsset
    $ytDlpSums = Join-Path $TempDir 'yt-dlp-SHA2-256SUMS'
    Download-File "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YtDlpAsset" $ytDlpPath
    Download-File 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS' $ytDlpSums
    Verify-Hash $ytDlpPath $YtDlpAsset $ytDlpSums
    Copy-Item -LiteralPath $ytDlpPath -Destination (Join-Path $InstallDir 'yt-dlp.exe.new') -Force
    Move-Item -LiteralPath (Join-Path $InstallDir 'yt-dlp.exe.new') -Destination (Join-Path $InstallDir 'yt-dlp.exe') -Force

    Write-Host "Downloading ytget-rs from https://github.com/$Repo/releases/latest ..."
    $appZip = Join-Path $TempDir $AppAsset
    $appSums = Join-Path $TempDir 'SHA256SUMS'
    Download-File "https://github.com/$Repo/releases/latest/download/$AppAsset" $appZip
    Download-File "https://github.com/$Repo/releases/latest/download/SHA256SUMS" $appSums
    Verify-Hash $appZip $AppAsset $appSums
    $appExtract = Join-Path $TempDir 'app'
    Expand-Archive -LiteralPath $appZip -DestinationPath $appExtract -Force
    $appExe = Join-Path $appExtract 'ytget-rs.exe'
    if (-not (Test-Path -LiteralPath $appExe)) { throw 'The release archive did not contain ytget-rs.exe.' }
    Copy-Item -LiteralPath $appExe -Destination (Join-Path $InstallDir 'ytget-rs.exe.new') -Force
    Move-Item -LiteralPath (Join-Path $InstallDir 'ytget-rs.exe.new') -Destination (Join-Path $InstallDir 'ytget-rs.exe') -Force

    # Persist the per-user PATH without requiring administrator access.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathEntries = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if (-not ($pathEntries | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') })) {
        $newPath = (@($pathEntries) + $InstallDir) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }
    if ($env:PATH -notlike "*$InstallDir*") { $env:PATH = "$InstallDir;$env:PATH" }

    Write-Host ''
    Write-Host "$(if ($Update) { 'Update' } else { 'Installation' }) complete." -ForegroundColor Green
    Write-Host "App:    $(Join-Path $InstallDir 'ytget-rs.exe')"
    Write-Host "yt-dlp: $(Join-Path $InstallDir 'yt-dlp.exe')"
    Write-Host "FFmpeg: $(Join-Path $InstallDir 'ffmpeg.exe')"
    Write-Host 'Open a new terminal, then run: ytget-rs'
}
finally {
    Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}
