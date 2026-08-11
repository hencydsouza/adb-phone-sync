$ErrorActionPreference = "Stop"
$zipPath = "$env:USERPROFILE\Downloads\platform-tools-latest-windows.zip"
$dest = "src-tauri/binaries"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

Expand-Archive -Path $zipPath -DestinationPath "$env:TEMP\platform-tools-extract" -Force
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\adb.exe" "$dest\adb-x86_64-pc-windows-msvc.exe"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinApi.dll" "$dest\AdbWinApi.dll"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinUsbApi.dll" "$dest\AdbWinUsbApi.dll"

Write-Host "Placed adb.exe + DLLs in $dest"
