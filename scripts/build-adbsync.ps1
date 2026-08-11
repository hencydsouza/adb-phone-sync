# Run from the repo root — all paths below are relative to it.
$ErrorActionPreference = "Stop"

pip install "pyinstaller==6.22.0"
if ($LASTEXITCODE -ne 0) {
    Write-Error "pip install pyinstaller failed (exit code $LASTEXITCODE)"
    exit $LASTEXITCODE
}

# --noconfirm: pyinstaller is a native exe, not a PowerShell cmdlet, so
# $ErrorActionPreference does NOT catch its non-zero exit codes, and without
# --noconfirm it prompts on stdin (hangs non-interactively) when dist/build
# from a previous run already exist.
pyinstaller `
  --onefile `
  --noconfirm `
  --name adbsync `
  --paths third_party/better-adb-sync/src `
  scripts/adbsync_entry.py
if ($LASTEXITCODE -ne 0) {
    Write-Error "pyinstaller build failed (exit code $LASTEXITCODE)"
    exit $LASTEXITCODE
}

$target = "src-tauri/binaries/adbsync-x86_64-pc-windows-msvc.exe"
New-Item -ItemType Directory -Force -Path "src-tauri/binaries" | Out-Null
Copy-Item -Force "dist/adbsync.exe" $target

Write-Host "Built $target"
