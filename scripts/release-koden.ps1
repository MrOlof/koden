# release-koden.ps1 — publish a Koden release via CI (the only sanctioned way).
# GitHub Actions (release.yml) builds, signs (key + password live as repo
# secrets — the local key backup is scrypt-locked and CI is its only reader),
# and publishes a DRAFT release on tag push; this script tags, pushes, waits,
# and undrafts. Bump `version` in src-tauri/tauri.conf.json first.
$ErrorActionPreference = "Stop"

$ver = (Get-Content src-tauri/tauri.conf.json | ConvertFrom-Json).version
$tag = "v$ver"

git push origin main
git tag $tag
git push origin $tag
Write-Host "tag $tag pushed - CI is building (github.com/MrOlof/koden/actions)"

gh run watch --repo MrOlof/koden --exit-status (gh run list --repo MrOlof/koden --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId')
gh release edit $tag --repo MrOlof/koden --draft=false
Write-Host "released $tag - installs pick it up on their next update check"
