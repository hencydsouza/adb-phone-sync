"""PyInstaller entry point for the vendored better-adb-sync package."""
import sys
# Path is relative to the repo root — run scripts/build-adbsync.ps1 from there.
sys.path.insert(0, "third_party/better-adb-sync/src")

from BetterADBSync import main

if __name__ == "__main__":
    main()
