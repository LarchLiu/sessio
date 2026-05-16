<#
.SYNOPSIS
  Bump version, commit, tag (Windows / PowerShell).

.DESCRIPTION
  Mirror of scripts/release.sh. Touches:
    package.json                  "version": "..."
    src-tauri/Cargo.toml          version = "..."
    src-tauri/tauri.conf.json     "version": "..."
    src-tauri/Cargo.lock          via `cargo check`

  Then commits + tags vX.Y.Z locally. Does NOT push; review and push:
    git push origin <branch> vX.Y.Z

.EXAMPLE
  .\scripts\release.ps1 0.2.0
#>

param(
    [Parameter(Position = 0)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$packageJson = Join-Path $root 'package.json'
$cargoToml = Join-Path $root 'src-tauri/Cargo.toml'
$tauriConf = Join-Path $root 'src-tauri/tauri.conf.json'

$releaseTag = (git -C $root tag --list 'v*' --sort=-v:refname | Select-Object -First 1).Trim()

if (-not $PSBoundParameters.ContainsKey('Version') -or [string]::IsNullOrWhiteSpace($Version)) {
    Write-Error @"
missing version
latest release tag: $(if ($releaseTag) { $releaseTag } else { '<none>' })
usage: .\scripts\release.ps1 0.2.0
"@
    exit 2
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Error "version must look like X.Y.Z (got '$Version')"
    exit 2
}

if ($releaseTag) {
    $latestReleaseVersion = $releaseTag -replace '^v', ''
    if ($latestReleaseVersion -notmatch '^\d+\.\d+\.\d+$') {
        Write-Error "latest release tag '$releaseTag' is not in vX.Y.Z format"
        exit 1
    }

    $newVersionValue = [version]$Version
    $latestVersionValue = [version]$latestReleaseVersion
    if ($newVersionValue -le $latestVersionValue) {
        Write-Error "version '$Version' must be greater than latest release tag '$releaseTag'"
        exit 1
    }
}

# Refuse if the working tree has uncommitted work; the bump commit below
# should contain only release version metadata.
if (git -C $root status --porcelain) {
    Write-Error "working tree has uncommitted changes; commit or stash first"
    exit 1
}

# Refuse if the tag already exists; otherwise `git tag` fails after the
# bump commit lands, leaving a commit to clean up by hand.
if (git -C $root tag -l "v$Version") {
    Write-Error "tag v$Version already exists"
    exit 1
}

(Get-Content $packageJson) `
    -replace '"version":\s*".*"', "`"version`": `"$Version`"" |
    Set-Content $packageJson -NoNewline:$false

(Get-Content $cargoToml) `
    -replace '^version = ".*"', "version = `"$Version`"" |
    Set-Content $cargoToml -NoNewline:$false

(Get-Content $tauriConf) `
    -replace '"version":\s*".*"', "`"version`": `"$Version`"" |
    Set-Content $tauriConf -NoNewline:$false

Push-Location (Join-Path $root 'src-tauri')
try { cargo check } finally { Pop-Location }

git -C $root add `
    package.json `
    src-tauri/Cargo.toml `
    src-tauri/tauri.conf.json `
    src-tauri/Cargo.lock
git -C $root commit -m "chore: release v$Version"
git -C $root tag "v$Version"

$branch = (git -C $root rev-parse --abbrev-ref HEAD).Trim()
Write-Host ""
Write-Host "release v$Version staged on branch '$branch'."
Write-Host "to publish (triggers .github/workflows/release.yml):"
Write-Host "    git push origin $branch v$Version"
