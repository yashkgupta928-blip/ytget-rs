$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Repo = $env:YTGET_REPO
if ([string]::IsNullOrWhiteSpace($Repo)) { $Repo = 'yashkgupta928-blip/ytget-rs' }
if ($Repo -like '*YOUR_GITHUB_USERNAME*' -or $Repo -notmatch '^[^/]+/[^/]+$') {
    throw 'Set $env:YTGET_REPO to owner/repository or set YTGET_REPO to a different repository if using a fork.'
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
