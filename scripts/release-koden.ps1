# release-koden.ps1 — build, sign, and publish a Koden release in one go.
# Run from the repo root on HQ (the machine holding the updater key):
#   pwsh scripts/release-koden.ps1
# Version comes from src-tauri/tauri.conf.json — bump it first.
$ErrorActionPreference = "Stop"

$ver = (Get-Content src-tauri/tauri.conf.json | ConvertFrom-Json).version
$key = "$HOME\Snorlax\_ClaudeSetup\secrets\koden-updater.key"
if (-not (Test-Path $key)) { throw "updater key not found at $key" }
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $key -Raw)

npx tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$dir = "src-tauri/target/release/bundle/nsis"
$exe = "$dir/Koden_${ver}_x64-setup.exe"
$sig = (Get-Content "$exe.sig" -Raw).Trim()

@{
    version   = $ver
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{
        "windows-x86_64" = @{
            signature = $sig
            url       = "https://github.com/MrOlof/koden/releases/download/v$ver/Koden_${ver}_x64-setup.exe"
        }
    }
} | ConvertTo-Json -Depth 4 | Set-Content "$dir/latest.json" -Encoding ascii

git push origin main
gh release create "v$ver" $exe "$dir/latest.json" --repo MrOlof/koden --title "Koden v$ver" --generate-notes
Write-Host "released v$ver - installed apps pick it up on next launch/check"
