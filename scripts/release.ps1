<#
.SYNOPSIS
  Bump version, commit, tag (Windows / PowerShell).

.DESCRIPTION
  Mirror of scripts/release.sh. Touches:
    package.json                  "version": "..."
    src-tauri/Cargo.toml          version = "..."
    src-tauri/tauri.conf.json     "version": "..."
    src-tauri/Cargo.lock          via `cargo check`

  Then commits + tags vX.Y.Z[-prerelease] locally. Does NOT push; review and push:
    git push origin <branch> vX.Y.Z[-prerelease]

.EXAMPLE
  .\scripts\release.ps1 0.2.0

.EXAMPLE
  .\scripts\release.ps1 0.3.0-beta.1
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

function Parse-SemVer([string]$Value) {
    $match = [regex]::Match($Value, '^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*))?$')
    if (-not $match.Success) {
        throw "invalid version: $Value"
    }
    [pscustomobject]@{
        Major = [int]$match.Groups[1].Value
        Minor = [int]$match.Groups[2].Value
        Patch = [int]$match.Groups[3].Value
        Prerelease = $match.Groups[4].Value
    }
}

function Compare-Prerelease([string]$Left, [string]$Right) {
    if (-not $Left -and -not $Right) { return 0 }
    if (-not $Left) { return 1 }
    if (-not $Right) { return -1 }
    $leftParts = $Left -split '\.'
    $rightParts = $Right -split '\.'
    $length = [Math]::Max($leftParts.Length, $rightParts.Length)
    for ($i = 0; $i -lt $length; $i++) {
        if ($i -ge $leftParts.Length) { return -1 }
        if ($i -ge $rightParts.Length) { return 1 }
        $leftIsNumber = $leftParts[$i] -match '^\d+$'
        $rightIsNumber = $rightParts[$i] -match '^\d+$'
        if ($leftIsNumber -and $rightIsNumber) {
            $diff = [int]$leftParts[$i] - [int]$rightParts[$i]
            if ($diff -ne 0) { return [Math]::Sign($diff) }
        } elseif ($leftIsNumber -ne $rightIsNumber) {
            if ($leftIsNumber) { return -1 }
            return 1
        } else {
            $diff = [string]::CompareOrdinal($leftParts[$i], $rightParts[$i])
            if ($diff -ne 0) { return [Math]::Sign($diff) }
        }
    }
    return 0
}

function Compare-SemVer($Left, $Right) {
    foreach ($part in @('Major', 'Minor', 'Patch')) {
        $diff = $Left.$part - $Right.$part
        if ($diff -ne 0) { return [Math]::Sign($diff) }
    }
    Compare-Prerelease $Left.Prerelease $Right.Prerelease
}

$releaseTag = ''
$releaseVersion = $null
foreach ($tagRow in (git -C $root tag --list 'v*')) {
    $tag = "$tagRow".Trim()
    if (-not $tag) { continue }
    try {
        $parsed = Parse-SemVer ($tag -replace '^v', '')
    } catch {
        continue
    }
    if (-not $releaseVersion -or (Compare-SemVer $parsed $releaseVersion) -gt 0) {
        $releaseTag = $tag
        $releaseVersion = $parsed
    }
}

if (-not $PSBoundParameters.ContainsKey('Version') -or [string]::IsNullOrWhiteSpace($Version)) {
    Write-Error @"
missing version
latest release tag: $(if ($releaseTag) { $releaseTag } else { '<none>' })
usage: .\scripts\release.ps1 0.2.0 or .\scripts\release.ps1 0.3.0-beta.1
"@
    exit 2
}

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$') {
    Write-Error "version must look like X.Y.Z or X.Y.Z-prerelease (got '$Version')"
    exit 2
}

if ($releaseTag) {
    $latestReleaseVersion = $releaseTag -replace '^v', ''
    if ($latestReleaseVersion -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$') {
        Write-Error "latest release tag '$releaseTag' is not in vX.Y.Z[-prerelease] format"
        exit 1
    }

    $newVersionValue = Parse-SemVer $Version
    $latestVersionValue = Parse-SemVer $latestReleaseVersion
    if ((Compare-SemVer $newVersionValue $latestVersionValue) -le 0) {
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
