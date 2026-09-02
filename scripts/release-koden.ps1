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

# Guard against a half-release before undrafting (v0.11.5: a create-release
# race put the Windows assets in an orphan draft; the published release had
# no exe and a latest.json without a windows entry).
$releaseCount = gh api repos/MrOlof/koden/releases --jq "[.[] | select(.tag_name == `"$tag`")] | length"
if ($releaseCount -ne "1") { throw "$releaseCount releases exist for $tag - merge them before undrafting" }
$assets = gh release view $tag --repo MrOlof/koden --json assets --jq "[.assets[].name] | join(`" `")"
foreach ($must in @("x64-setup.exe", "x64-setup.exe.sig", ".AppImage", "latest.json")) {
    if ($assets -notlike "*$must*") { throw "release $tag is missing $must - not undrafting" }
}
$tmp = Join-Path $env:TEMP "koden-latest-check"
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue; New-Item -ItemType Directory $tmp | Out-Null
gh release download $tag --repo MrOlof/koden --pattern latest.json --dir $tmp
if (-not (Select-String -Path (Join-Path $tmp "latest.json") -Pattern "windows-x86_64" -Quiet)) {
    throw "latest.json for $tag has no windows-x86_64 entry - not undrafting"
}

gh release edit $tag --repo MrOlof/koden --draft=false
Write-Host "released $tag - installs pick it up on their next update check"
