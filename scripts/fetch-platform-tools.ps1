$ErrorActionPreference = "Stop"
$zipPath = "$env:USERPROFILE\Downloads\platform-tools-latest-windows.zip"
$dest = "src-tauri/binaries"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

Expand-Archive -Path $zipPath -DestinationPath "$env:TEMP\platform-tools-extract" -Force
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\adb.exe" "$dest\adb-x86_64-pc-windows-msvc.exe"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinApi.dll" "$dest\AdbWinApi.dll"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinUsbApi.dll" "$dest\AdbWinUsbApi.dll"

# adb.exe is a native exe, not a PowerShell cmdlet, so $ErrorActionPreference
# does NOT catch its non-zero exit codes or a garbled/corrupt binary - verify
# the freshly-placed binary actually runs and reports itself as adb before
# declaring success, same pattern as build-adbsync.ps1's pyinstaller checks.
$adbExe = "$dest\adb-x86_64-pc-windows-msvc.exe"
$versionOutput = & $adbExe version 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Write-Error "adb version check failed (exit code $LASTEXITCODE): $versionOutput"
    exit $LASTEXITCODE
}
if ($versionOutput -notmatch "Android Debug Bridge") {
    Write-Error "adb version check did not report 'Android Debug Bridge' - binary may be corrupt or stale: $versionOutput"
    exit 1
}

Write-Host "Placed adb.exe + DLLs in $dest"
Write-Host "Verified: $($versionOutput.Trim())"
