# Generate app icon assets from the finalized logo master.
#
#   pwsh -File docs/src/assets/branding/app-icon/Generate-AppIcons.ps1
#
# Outputs (Windows):
#   docs/src/assets/branding/app-icon/windows/eiviz.ico          16..256 px, multi-size
#   docs/src/assets/branding/app-icon/macos/eiviz.iconset/*.png
#   docs/src/assets/branding/app-icon/macos/eiviz.icns           (via iconutil, macOS only)
#
# The icon is the framed 07-I3 (light) variant, which reads correctly on both
# light and dark shells. Re-run this after replacing the PNG in ../png.

param(
  [string]$Source = (Join-Path $PSScriptRoot '..\png\eiviz-07-I3-framed-icon.png'),
  # 0 = crop exactly to the frame; small positive values keep the rounded frame
  # edge from touching the canvas once it is scaled down.
  [double]$PadRatio = 0.03
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$Source = (Resolve-Path $Source).Path
$WinDir = Join-Path $PSScriptRoot 'windows'
$MacDir = Join-Path $PSScriptRoot 'macos'
$IconsetDir = Join-Path $MacDir 'eiviz.iconset'
New-Item -ItemType Directory -Force -Path $WinDir, $MacDir, $IconsetDir | Out-Null

# --- render a square PNG at the requested size -------------------------------
function New-IconPng {
  param([int]$Size, [string]$OutPath, [double]$Opacity = 1.0)
  $src = [System.Drawing.Image]::FromFile($Source)
  $bmp = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
  $g.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceOver
  $g.Clear([System.Drawing.Color]::Transparent)
  $pad = [int][Math]::Round($Size * $PadRatio)
  $dst = New-Object System.Drawing.Rectangle($pad, $pad, ($Size - 2 * $pad), ($Size - 2 * $pad))
  if ($Opacity -ge 1.0) {
    $g.DrawImage($src, $dst)
  } else {
    $attr = New-Object System.Drawing.Imaging.ImageAttributes
    $cm = New-Object System.Drawing.Imaging.ColorMatrix
    $cm.Matrix33 = [float]$Opacity
    $attr.SetColorMatrix($cm)
    $g.DrawImage($src, $dst, 0, 0, $src.Width, $src.Height, [System.Drawing.GraphicsUnit]::Pixel, $attr)
    $attr.Dispose()
  }
  $g.Dispose(); $src.Dispose()
  $bmp.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

# --- 1. build the multi-size .ico -------------------------------------------
# DIB (BMP3) entries are used below 256 px for maximum shell compatibility and
# PNG compression for the 256 px entry, as Windows recommends.
$sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
$entries = @()
foreach ($s in $sizes) {
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("eiviz-icon-{0}.png" -f $s)
  New-IconPng -Size $s -OutPath $tmp
  if ($s -eq 256) {
    $payload = [System.IO.File]::ReadAllBytes($tmp)
    $entries += [pscustomobject]@{ Size = $s; Png = $true; Data = $payload }
  } else {
    # Convert the PNG to a 32bpp bottom-up DIB with an AND mask.
    $bmp = New-Object System.Drawing.Bitmap($tmp)
    $rect = New-Object System.Drawing.Rectangle(0, 0, $s, $s)
    $locked = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $stride = $locked.Stride
    $raw = New-Object byte[] ($stride * $s)
    [System.Runtime.InteropServices.Marshal]::Copy($locked.Scan0, $raw, 0, $raw.Length)
    $bmp.UnlockBits($locked); $bmp.Dispose()

    $maskStride = [int]([Math]::Ceiling($s / 32.0) * 4)
    $mask = New-Object byte[] ($maskStride * $s)
    $xorSize = $s * $s * 4
    $dib = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter($dib)
    $w.Write([int]40)            # biSize
    $w.Write([int]$s)            # biWidth
    $w.Write([int]($s * 2))      # biHeight (XOR + AND)
    $w.Write([int16]1)           # biPlanes
    $w.Write([int16]32)          # biBitCount
    $w.Write([int]0)             # biCompression = BI_RGB
    $w.Write([int]($xorSize + $mask.Length))
    $w.Write([int]0); $w.Write([int]0); $w.Write([int]0); $w.Write([int]0)
    for ($y = $s - 1; $y -ge 0; $y--) {
      for ($x = 0; $x -lt $s; $x++) {
        $o = $y * $stride + $x * 4
        $b = $raw[$o]; $g = $raw[$o + 1]; $r = $raw[$o + 2]; $a = $raw[$o + 3]
        # BGRA premultiplied by nothing; fully transparent pixels go to 0.
        if ($a -eq 0) { $w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0); $w.Write([byte]0) }
        else { $w.Write([byte]$b); $w.Write([byte]$g); $w.Write([byte]$r); $w.Write([byte]$a) }
      }
    }
    for ($y = $s - 1; $y -ge 0; $y--) {
      $rowStart = $w.BaseStream.Position
      for ($byteIdx = 0; $byteIdx -lt $maskStride; $byteIdx++) {
        $acc = 0
        for ($bit = 0; $bit -lt 8; $bit++) {
          $x = $byteIdx * 8 + $bit
          if ($x -lt $s) {
            $alpha = $raw[$y * $stride + $x * 4 + 3]
            if ($alpha -eq 0) { $acc = $acc -bor (0x80 -shr $bit) }
          }
        }
        $w.Write([byte]$acc)
      }
    }
    $w.Flush()
    $entries += [pscustomobject]@{ Size = $s; Png = $false; Data = $dib.ToArray() }
    $w.Dispose(); $dib.Dispose()
  }
  Remove-Item $tmp -Force
}

$icoPath = Join-Path $WinDir 'eiviz.ico'
$fs = [System.IO.File]::Create($icoPath)
$bw = New-Object System.IO.BinaryWriter($fs)
$bw.Write([int16]0); $bw.Write([int16]1); $bw.Write([int16]$entries.Count)
$offset = 6 + 16 * $entries.Count
foreach ($e in $entries) {
  $dim = if ($e.Size -ge 256) { 0 } else { $e.Size }
  $bw.Write([byte]$dim); $bw.Write([byte]$dim)
  $bw.Write([byte]0); $bw.Write([byte]0)   # palette, reserved
  $bw.Write([int16]1); $bw.Write([int16]32) # planes, bpp
  $bw.Write([int]$e.Data.Length)
  $bw.Write([int]$offset)
  $offset += $e.Data.Length
}
foreach ($e in $entries) { $bw.Write($e.Data) }
$bw.Flush(); $bw.Dispose(); $fs.Dispose()
Write-Host ("ico  -> {0} ({1} sizes: {2})" -f $icoPath, $entries.Count, ($sizes -join ','))

# --- 2. build the .icns iconset ---------------------------------------------
$iconset = @(
  @{ Name = 'icon_16x16.png'; Size = 16 },
  @{ Name = 'icon_16x16@2x.png'; Size = 32 },
  @{ Name = 'icon_32x32.png'; Size = 32 },
  @{ Name = 'icon_32x32@2x.png'; Size = 64 },
  @{ Name = 'icon_128x128.png'; Size = 128 },
  @{ Name = 'icon_128x128@2x.png'; Size = 256 },
  @{ Name = 'icon_256x256.png'; Size = 256 },
  @{ Name = 'icon_256x256@2x.png'; Size = 512 },
  @{ Name = 'icon_512x512.png'; Size = 512 },
  @{ Name = 'icon_512x512@2x.png'; Size = 1024 }
)
Get-ChildItem $IconsetDir -Filter *.png | Remove-Item -Force
foreach ($i in $iconset) { New-IconPng -Size $i.Size -OutPath (Join-Path $IconsetDir $i.Name) }
Write-Host ("iconset -> {0} ({1} images)" -f $IconsetDir, $iconset.Count)

$icnsPath = Join-Path $MacDir 'eiviz.icns'
if (Get-Command iconutil -ErrorAction SilentlyContinue) {
  if (Test-Path $icnsPath) { Remove-Item $icnsPath -Force }
  iconutil -c icns $IconsetDir -o $icnsPath
  if ($LASTEXITCODE -ne 0) { throw "iconutil failed with exit code $LASTEXITCODE" }
  Write-Host "icns -> $icnsPath"
} else {
  Write-Host "icns -> skipped: iconutil is only available on macOS. Run this script on a Mac, or commit the generated eiviz.icns."
}
