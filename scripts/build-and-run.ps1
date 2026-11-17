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
# Generated for this workspace only; a release build uses the real module key. The mixer
# embeds the public key and the signing tool takes the matching secret; a module signed
# with any other pair is refused at startup, so these two travel together.
$env:EIVIZ_PRO_MODULE_PUBLIC_KEY = "dbfd58007f385d1bd3ece7f3abb39c9a18356e273bb083fe77d527c03ca96836"
$env:EIVIZ_PRO_MODULE_SECRET = "9d0ad273e4efb359bc80b55ec4361c0af3bea46acf813bc48cf5792d4e3b094b"

$root = "D:\eiviz\eiviz"
$proRoot = "D:\eiviz\eiviz-pro"
$rel = Join-Path $root "hosts\win32\bin\Release\net10.0-windows"

Get-Process Eiviz.Host -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

Push-Location $root
try {
    # The public key is read by option_env!, so cargo does not rebuild when it changes.
    # Touching the file that reads it is what makes the cached build pick the key up.
    (Get-Item "crates/eiviz-mixer/src/pro_load.rs").LastWriteTime = Get-Date
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

# The host build does not build the Pro module, and a fresh build drops the signature the
# mixer checks. Build it with the dev plan switch so EIVIZ_PLAN can pick the plan (a release
# module without it always runs Community, which caps RTMP at 720p30), sign it, and deploy
# the DLL with its manifest. The module workspace has its own target directory.
$hostTarget = $env:CARGO_TARGET_DIR
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
Push-Location $proRoot
try {
    cargo build -p eiviz_pro --release --features pro-dev --locked
    if ($LASTEXITCODE -ne 0) { throw "pro module build failed" }
    & (Join-Path $proRoot "target\release\eiviz-pro-ticket.exe") sign-module `
        --module (Join-Path $proRoot "target\release\eiviz_pro.dll")
    if ($LASTEXITCODE -ne 0) { throw "signing the Pro module failed" }
}
finally {
    Pop-Location
    $env:CARGO_TARGET_DIR = $hostTarget
}
Copy-Item (Join-Path $proRoot "target\release\eiviz_pro.dll") (Join-Path $rel "eiviz_pro.dll") -Force
Copy-Item (Join-Path $proRoot "target\release\eiviz_pro.dll.manifest") (Join-Path $rel "eiviz_pro.dll.manifest") -Force
Set-Content (Join-Path $rel "eiviz-pro.required") "eiviz_pro.dll" -NoNewline

# `EIVIZ_PLAN` is read by the module, so it has to be in the environment of the child
# process. `Start-Process -Environment` only exists on PowerShell 7.
$env:EIVIZ_PLAN = "professional"
Start-Process -FilePath (Join-Path $rel "Eiviz.Host.exe")
Start-Sleep -Seconds 9
$app = Get-Process Eiviz.Host -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $app) { throw "the host did not start" }
"running: $($app.Id) $($app.MainWindowTitle)"
