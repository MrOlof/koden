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

# Wait for THIS tag's run to register (a bare `--limit 1` races the push and
# can grab the previous release's run).
$runId = $null
for ($i = 0; $i -lt 12 -and -not $runId; $i++) {
    Start-Sleep -Seconds 10
    $runId = gh run list --repo MrOlof/koden --workflow=release.yml --json databaseId,headBranch --jq "[.[] | select(.headBranch == `"$tag`")][0].databaseId"
}
if (-not $runId) { throw "CI run for $tag never appeared" }
gh run watch $runId --repo MrOlof/koden --exit-status
gh release edit $tag --repo MrOlof/koden --draft=false
Write-Host "released $tag - installs pick it up on their next update check"
