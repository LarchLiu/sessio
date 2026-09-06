[CmdletBinding()]
param(
  [Parameter(Position = 0, Mandatory = $true)]
  [string]$SourceAppDir,

  [Parameter(Position = 1, Mandatory = $true)]
  [string]$AppSlug,

  [switch]$Update
)

$ErrorActionPreference = 'Stop'

function Fail([int]$Code, [string]$Message) {
  [Console]::Error.WriteLine($Message)
  exit $Code
}

function Merge-AppTree([string]$Source, [string]$Destination) {
  if (-not (Test-Path -LiteralPath $Destination)) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
  }

  foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
    $target = Join-Path $Destination $item.Name
    $isRealDirectory = $item.PSIsContainer -and [string]::IsNullOrEmpty($item.LinkType)
    if ($isRealDirectory) {
      if (Test-Path -LiteralPath $target) {
        $targetItem = Get-Item -LiteralPath $target -Force
        $targetIsRealDirectory = $targetItem.PSIsContainer -and [string]::IsNullOrEmpty($targetItem.LinkType)
        if (-not $targetIsRealDirectory) {
          Remove-Item -LiteralPath $target -Recurse -Force
          New-Item -ItemType Directory -Path $target -Force | Out-Null
        }
      } else {
        New-Item -ItemType Directory -Path $target -Force | Out-Null
      }
      Merge-AppTree $item.FullName $target
    } else {
      if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Recurse -Force
      }
      Copy-Item -LiteralPath $item.FullName -Destination $target -Force
    }
  }
}

function Write-ClaudeInstructions([string]$Destination) {
  $agentsFile = Join-Path $Destination 'AGENTS.md'
  if (Test-Path -LiteralPath $agentsFile -PathType Leaf) {
    Copy-Item -LiteralPath $agentsFile -Destination (Join-Path $Destination 'CLAUDE.md') -Force
  }
}

$appHome = [Environment]::GetEnvironmentVariable('SESSIO_APP_HOME')
if ([string]::IsNullOrWhiteSpace($appHome)) {
  Fail 78 'SESSIO_APP_HOME is not set; refusing to guess a Sessio profile.'
}
if (-not [System.IO.Path]::IsPathRooted($appHome)) {
  Fail 78 'SESSIO_APP_HOME must be an absolute path.'
}
if (-not (Test-Path -LiteralPath $SourceAppDir -PathType Container)) {
  Fail 66 "Source app directory does not exist: $SourceAppDir"
}
if ($AppSlug -notmatch '^[a-z0-9]+([.-][a-z0-9]+)*$') {
  Fail 64 "App slug must use lowercase ASCII segments: $AppSlug"
}

$source = (Resolve-Path -LiteralPath $SourceAppDir).Path
$appsDir = Join-Path $appHome 'apps'
$destination = Join-Path $appsDir $AppSlug
if ((Test-Path -LiteralPath $destination) -and -not $Update) {
  Fail 73 "Destination already exists; inspect it or rerun with -Update: $destination"
}
if ((Test-Path -LiteralPath $destination) -and $Update) {
  $destinationItem = Get-Item -LiteralPath $destination -Force
  $destinationIsRealDirectory = $destinationItem.PSIsContainer -and [string]::IsNullOrEmpty($destinationItem.LinkType)
  if (-not $destinationIsRealDirectory) {
    Fail 73 "Existing destination must be a real directory: $destination"
  }
}

New-Item -ItemType Directory -Path $appsDir -Force | Out-Null
if ((Test-Path -LiteralPath $destination) -and $Update) {
  Merge-AppTree $source $destination
  Write-ClaudeInstructions $destination
  [Console]::Out.WriteLine($destination)
  exit 0
}

$staging = Join-Path $appsDir ('.{0}.publish.{1}' -f $AppSlug, [Guid]::NewGuid().ToString('N'))

try {
  New-Item -ItemType Directory -Path $staging -Force | Out-Null
  Merge-AppTree $source $staging
  Write-ClaudeInstructions $staging
  Move-Item -LiteralPath $staging -Destination $destination
  $staging = $null
  [Console]::Out.WriteLine($destination)
} finally {
  if ($staging -and (Test-Path -LiteralPath $staging)) {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
  }
}
