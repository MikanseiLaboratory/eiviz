param(
  [string]$IconsetDir = (Join-Path $PSScriptRoot 'macos\eiviz.iconset'),
  [string]$OutPath    = (Join-Path $PSScriptRoot 'macos\eiviz.icns')
)
# Encode a minimal but standard ICNS container from an Apple .iconset.
# Each entry is a PNG blob under its ICNS type; modern macOS (10.10+) reads
# PNG payloads for every type used here.
#
# This is the cross-platform fallback for `iconutil -c icns` (macOS only).
# Prefer `pwsh -File Generate-AppIcons.ps1` on macOS so the file matches what
# Apple's own tool produces.

$ErrorActionPreference = 'Stop'
$typeByFile = @{
  'icon_16x16.png'      = 'icp4'  # 16x16
  'icon_16x16@2x.png'   = 'icp5'  # 32x32
  'icon_32x32.png'      = 'icp5'  # 32x32
  'icon_32x32@2x.png'   = 'icp6'  # 64x64
  'icon_128x128.png'    = 'ic07'  # 128x128
  'icon_128x128@2x.png' = 'ic08'  # 256x256
  'icon_256x256.png'    = 'ic08'  # 256x256
  'icon_256x256@2x.png' = 'ic09'  # 512x512
  'icon_512x512.png'    = 'ic09'  # 512x512
  'icon_512x512@2x.png' = 'ic10'  # 1024x1024 (512@2x)
}

$chunks = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter($chunks)
$seen = @{}
foreach ($file in $typeByFile.Keys | Sort-Object) {
  $path = Join-Path $IconsetDir $file
  if (-not (Test-Path $path)) { throw "missing iconset entry: $path" }
  $type = $typeByFile[$file]
  $bytes = [System.IO.File]::ReadAllBytes($path)
  $writer.Write([System.Text.Encoding]::ASCII.GetBytes($type))
  $writer.Write([int]([System.Net.IPAddress]::HostToNetworkOrder([int]($bytes.Length + 8))))
  $writer.Write($bytes)
  $seen[$type] = $true
}
$writer.Flush()

$body = $chunks.ToArray()
$writer.Dispose(); $chunks.Dispose()

$out = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($out)
$bw.Write([System.Text.Encoding]::ASCII.GetBytes('icns'))
$bw.Write([int]([System.Net.IPAddress]::HostToNetworkOrder([int]($body.Length + 8))))
$bw.Write($body)
$bw.Flush()
[System.IO.File]::WriteAllBytes($OutPath, $out.ToArray())
$bw.Dispose(); $out.Dispose()

Write-Host ("icns -> {0} ({1} bytes, types: {2})" -f $OutPath, (Get-Item $OutPath).Length, (($seen.Keys | Sort-Object) -join ','))
