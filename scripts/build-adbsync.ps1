$ErrorActionPreference = "Stop"

pip install pyinstaller

pyinstaller `
  --onefile `
  --name adbsync `
  --paths third_party/better-adb-sync/src `
  scripts/adbsync_entry.py

$target = "src-tauri/binaries/adbsync-x86_64-pc-windows-msvc.exe"
New-Item -ItemType Directory -Force -Path "src-tauri/binaries" | Out-Null
Copy-Item -Force "dist/adbsync.exe" $target

Write-Host "Built $target"
