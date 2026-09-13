[CmdletBinding()]
param(
  [Parameter(Position = 0, Mandatory = $true)]
  [string]$SourceAppDir,

  [Parameter(Position = 1, Mandatory = $true)]
  [string]$AppSlug,

  [switch]$Update,

  [switch]$UpdateData
)

$ErrorActionPreference = 'Stop'

function Fail([int]$Code, [string]$Message) {
  [Console]::Error.WriteLine($Message)
  exit $Code
}

function Merge-AppTree([string]$Source, [string]$Destination, [string]$RelativePath = '', [bool]$StageDataUpdate = $false) {
  if (-not (Test-Path -LiteralPath $Destination)) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
  }

  foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
    $target = Join-Path $Destination $item.Name
    $itemRelativePath = if ([string]::IsNullOrEmpty($RelativePath)) { $item.Name } else { Join-Path $RelativePath $item.Name }
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
      Merge-AppTree $item.FullName $target $itemRelativePath $StageDataUpdate
    } else {
      if (-not $UpdateData -and $itemRelativePath -eq (Join-Path 'web' "${AppSlug}-data.js") -and (Test-Path -LiteralPath $target -PathType Leaf)) {
        if ($StageDataUpdate) {
          $pendingTarget = Join-Path ([System.IO.Path]::GetDirectoryName($target)) "${AppSlug}-data.pending.js"
          if (Test-Path -LiteralPath $pendingTarget) {
            Remove-Item -LiteralPath $pendingTarget -Recurse -Force
          }
          Copy-Item -LiteralPath $item.FullName -Destination $pendingTarget -Force
        }
        continue
      }
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
$manifestName = '.sessio-publish-manifest'
if ($UpdateData) {
  $Update = $true
}
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
$stageDataUpdate = $Update -and -not $UpdateData -and (Test-Path -LiteralPath (Join-Path $source 'web' "${AppSlug}-migrations.js") -PathType Leaf)

function Get-ManagedPaths([string]$Root) {
  Get-ChildItem -LiteralPath $Root -Recurse -Force | Where-Object {
    -not $_.PSIsContainer -and $_.Name -ne $manifestName
  } | ForEach-Object {
    $_.FullName.Substring($Root.Length).TrimStart([char]'\', [char]'/').Replace('\', '/')
  } | Sort-Object
}

function Write-PublishManifest([string]$Target, [string]$Root) {
  $paths = [System.Collections.Generic.List[string]]::new()
  [void]$paths.Add('version=1')
  foreach ($path in Get-ManagedPaths $Root) {
    [void]$paths.Add($path)
  }
  if (-not $UpdateData -and (Test-Path -LiteralPath (Join-Path $Target 'web' "${AppSlug}-data.js") -PathType Leaf)) {
    [void]$paths.Add("web/${AppSlug}-data.js")
  }
  if (Test-Path -LiteralPath (Join-Path $Root 'AGENTS.md') -PathType Leaf) {
    [void]$paths.Add('CLAUDE.md')
  }
  $paths | Sort-Object -Unique | Set-Content -LiteralPath (Join-Path $Target $manifestName) -Encoding UTF8
}

function Reconcile-ManagedPaths([string]$Root, [string]$Destination) {
  $oldManifest = Join-Path $Destination $manifestName
  if (-not (Test-Path -LiteralPath $oldManifest -PathType Leaf)) {
    return
  }
  $current = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($path in Get-ManagedPaths $Root) {
    [void]$current.Add($path)
  }
  if (-not $UpdateData) {
    [void]$current.Add("web/${AppSlug}-data.js")
  }
  if (Test-Path -LiteralPath (Join-Path $Root 'AGENTS.md') -PathType Leaf) {
    [void]$current.Add('CLAUDE.md')
  }
  foreach ($oldPath in Get-Content -LiteralPath $oldManifest) {
    if ([string]::IsNullOrWhiteSpace($oldPath) -or $oldPath -like 'version=*' -or $current.Contains($oldPath)) {
      continue
    }
    $targetPath = Join-Path $Destination ($oldPath.Replace('/', [System.IO.Path]::DirectorySeparatorChar))
    if (Test-Path -LiteralPath $targetPath) {
      Remove-Item -LiteralPath $targetPath -Recurse -Force
    }
  }
}

New-Item -ItemType Directory -Path $appsDir -Force | Out-Null
if ((Test-Path -LiteralPath $destination) -and $Update) {
  Reconcile-ManagedPaths $source $destination
  Merge-AppTree $source $destination '' $stageDataUpdate
  Write-PublishManifest $destination $source
  Write-ClaudeInstructions $destination
  [Console]::Out.WriteLine($destination)
  exit 0
}

$staging = Join-Path $appsDir ('.{0}.publish.{1}' -f $AppSlug, [Guid]::NewGuid().ToString('N'))

try {
  New-Item -ItemType Directory -Path $staging -Force | Out-Null
  Merge-AppTree $source $staging '' $stageDataUpdate
  Write-ClaudeInstructions $staging
  Write-PublishManifest $staging $source
  Move-Item -LiteralPath $staging -Destination $destination
  $staging = $null
  [Console]::Out.WriteLine($destination)
} finally {
  if ($staging -and (Test-Path -LiteralPath $staging)) {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
  }
}
