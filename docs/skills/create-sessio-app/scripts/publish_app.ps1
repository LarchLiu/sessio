[CmdletBinding()]
param(
  [Parameter(Position = 0, Mandatory = $true)]
  [string]$SourceAppDir,

  [Parameter(Position = 1, Mandatory = $true)]
  [string]$AppSlug,

  [switch]$Force
)

$ErrorActionPreference = 'Stop'

function Fail([int]$Code, [string]$Message) {
  [Console]::Error.WriteLine($Message)
  exit $Code
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
if ((Test-Path -LiteralPath $destination) -and -not $Force) {
  Fail 73 "Destination already exists; inspect it or rerun with -Force: $destination"
}

New-Item -ItemType Directory -Path $appsDir -Force | Out-Null
$staging = Join-Path $appsDir ('.{0}.publish.{1}' -f $AppSlug, [Guid]::NewGuid().ToString('N'))

try {
  New-Item -ItemType Directory -Path $staging -Force | Out-Null
  Get-ChildItem -LiteralPath $source -Force | Copy-Item -Destination $staging -Recurse -Force

  if (Test-Path -LiteralPath $destination) {
    Remove-Item -LiteralPath $destination -Recurse -Force
  }
  Move-Item -LiteralPath $staging -Destination $destination
  $staging = $null
  [Console]::Out.WriteLine($destination)
} finally {
  if ($staging -and (Test-Path -LiteralPath $staging)) {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
  }
}
