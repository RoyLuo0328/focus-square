param([Parameter(Mandatory = $true)][string]$BinaryPath)

$certificateBase64 = $env:WINDOWS_CERTIFICATE_BASE64
$certificatePassword = $env:WINDOWS_CERTIFICATE_PASSWORD

if ([string]::IsNullOrWhiteSpace($certificateBase64)) {
    Write-Warning "Windows signing certificate not configured; build will be unsigned."
    exit 0
}

if ([string]::IsNullOrWhiteSpace($certificatePassword)) {
    throw "WINDOWS_CERTIFICATE_PASSWORD is required."
}

$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$certificatePath = Join-Path $temporaryRoot "focus-square-signing.pfx"
$signTool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (-not $signTool) {
    $signTool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName |
        Select-Object -Last 1
}
if (-not $signTool) {
    throw "signtool.exe was not found."
}
$signToolPath = if ($signTool.Source) { $signTool.Source } else { $signTool.FullName }

try {
    [System.IO.File]::WriteAllBytes($certificatePath, [Convert]::FromBase64String($certificateBase64))
    & $signToolPath sign /fd SHA256 /f $certificatePath /p $certificatePassword /tr http://timestamp.digicert.com /td SHA256 $BinaryPath
    if ($LASTEXITCODE -ne 0) { throw "signtool.exe failed with exit code $LASTEXITCODE." }
} finally {
    if (Test-Path $certificatePath) { Remove-Item $certificatePath -Force }
}
