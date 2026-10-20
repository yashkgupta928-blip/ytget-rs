$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Repo = $env:YTGET_REPO
if ([string]::IsNullOrWhiteSpace($Repo)) { $Repo = 'YOUR_GITHUB_USERNAME/ytget-rs' }
if ($Repo -like '*YOUR_GITHUB_USERNAME*' -or $Repo -notmatch '^[^/]+/[^/]+$') {
    throw 'Set $env:YTGET_REPO to owner/repository or replace YOUR_GITHUB_USERNAME/ytget-rs in install.ps1 and update.ps1 before publishing.'
}
$env:YTGET_REPO = $Repo
$installerUrl = "https://raw.githubusercontent.com/$Repo/main/install.ps1"
$tempInstaller = Join-Path $env:TEMP ("ytget-rs-installer-" + [guid]::NewGuid().ToString('N') + '.ps1')
try {
    Invoke-WebRequest -Uri $installerUrl -OutFile $tempInstaller -UseBasicParsing -Headers @{ 'User-Agent' = 'ytget-rs-updater' }
    & $tempInstaller -Update
}
finally {
    Remove-Item -LiteralPath $tempInstaller -Force -ErrorAction SilentlyContinue
}
