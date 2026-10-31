# Builds and deploys everything the running app needs, in the order that works.
#
# `dotnet build` copies the mixer out of the workspace target directory, which is a build
# without the Pro module's public key, so it has to be rebuilt with the key afterwards and
# copied over the host's copy. Doing that by hand is what left the app unable to start
# twice; this script keeps the order in one place.

$ErrorActionPreference = "Stop"

$env:CPAL_ASIO_DIR = "D:\eiviz\.cargo-target-review\asiosdk\asiosdk_2.3.3_2019-06-14"
$env:CARGO_TARGET_DIR = "D:\eiviz\.cargo-target-review"
$env:TMP = "D:\eiviz\.cargo-target-review\tmp"
$env:TEMP = $env:TMP
# Generated for this workspace only; a release build uses the real module key.
$env:EIVIZ_PRO_MODULE_PUBLIC_KEY = "dbfd58007f385d1bd3ece7f3abb39c9a18356e273bb083fe77d527c03ca96836"

$root = "D:\eiviz\eiviz"
$rel = Join-Path $root "hosts\win32\bin\Release\net10.0-windows"

Get-Process Eiviz.Host -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

Push-Location $root
try {
    # The public key is read by option_env!, so cargo does not rebuild when it changes.
    # Touching the file that reads it is what makes the cached build pick the key up.
    (Get-Item "mixer/src/pro_load.rs").LastWriteTime = Get-Date
    cargo build -p eiviz_mixer --release --locked
    if ($LASTEXITCODE -ne 0) { throw "mixer build failed" }

    dotnet build "hosts/win32/Eiviz.Host.csproj" -c Release --nologo
    if ($LASTEXITCODE -ne 0) { throw "host build failed" }
}
finally {
    Pop-Location
}

# After the host build, so the keyed mixer is the one in the output directory.
Copy-Item "D:\eiviz\.cargo-target-review\release\eiviz_mixer.dll" (Join-Path $rel "eiviz_mixer.dll") -Force

# The host build also rebuilds the Pro module, which drops the signature the mixer checks.
# Sign a copy somewhere writable — the output directory is not writable for the signing
# tool — and deploy that, so the module and its manifest always travel together.
$env:EIVIZ_PRO_MODULE_SECRET = "9d0ad273e4efb359bc80b55ec4361c0af3bea46acf813bc48cf5792d4e3b094b"
$sign = "D:\eiviz\.cargo-target-review\sign"
New-Item -ItemType Directory -Force -Path $sign | Out-Null
Copy-Item "D:\eiviz\.cargo-target-review\pro\release\eiviz_pro.dll" (Join-Path $sign "eiviz_pro.dll") -Force
Remove-Item (Join-Path $sign "eiviz_pro.dll.manifest") -Force -ErrorAction SilentlyContinue
& "D:\eiviz\.cargo-target-review\pro\debug\eiviz-pro-ticket.exe" sign-module --module (Join-Path $sign "eiviz_pro.dll")
if ($LASTEXITCODE -ne 0) { throw "signing the Pro module failed" }
Copy-Item (Join-Path $sign "eiviz_pro.dll") (Join-Path $rel "eiviz_pro.dll") -Force
Copy-Item (Join-Path $sign "eiviz_pro.dll.manifest") (Join-Path $rel "eiviz_pro.dll.manifest") -Force
Set-Content (Join-Path $rel "eiviz-pro.required") "eiviz_pro.dll" -NoNewline

Start-Process -FilePath (Join-Path $rel "Eiviz.Host.exe") -Environment @{ EIVIZ_PLAN = "professional" }
Start-Sleep -Seconds 9
$app = Get-Process Eiviz.Host -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $app) { throw "the host did not start" }
"running: $($app.Id) $($app.MainWindowTitle)"
