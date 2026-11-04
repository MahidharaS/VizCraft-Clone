# Builds and packages VizCraft for Windows:
#
#   dist\VizCraft-<version>-windows-x86_64.zip   vizcraft.exe, vizcraft-server.exe, README.txt
#   dist\VizCraft-<version>-windows-x86_64.msi   when the WiX Toolset v3 (candle/light) is on PATH
#
# Usage: pwsh packaging/windows/package.ps1 [-SkipBuild]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'

$Root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $Root
$Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($env:VIZCRAFT_VERSION) { $Version = $env:VIZCRAFT_VERSION }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Dist = Join-Path $Root 'dist'
$Name = "VizCraft-$Version-windows-x86_64"
$Work = Join-Path $TargetDir 'windows-package'
$Stage = Join-Path $Work $Name

Write-Host "==> VizCraft $Version for Windows"
if (-not $SkipBuild) {
    rustup target add wasm32-unknown-unknown | Out-Null
    cargo build -p vizcraft-wasm --release --target wasm32-unknown-unknown
    if ($LASTEXITCODE -ne 0) { throw 'wasm build failed' }
    cargo build --release -p vizcraft -p vizcraft-server -p vizcraft-mlserve
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
}

if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
New-Item -ItemType Directory -Force -Path $Stage, $Dist | Out-Null
Copy-Item (Join-Path $TargetDir 'release\vizcraft.exe') $Stage
Copy-Item (Join-Path $TargetDir 'release\vizcraft-server.exe') $Stage
Copy-Item (Join-Path $TargetDir 'release\vizcraft-mlserve.exe') $Stage
@"
VizCraft $Version for Windows

Run vizcraft.exe. Windows SmartScreen may warn that the app is unrecognised (it isn't code-signed):
choose "More info", then "Run anyway".

VizCraft Service:  vizcraft-server.exe serve --port 8080 --token SECRET
Model server:      vizcraft-mlserve.exe --port 8765
Docs: docs/publishing.md in the source repository.
"@ | Set-Content -Encoding UTF8 (Join-Path $Stage 'README.txt')

$Zip = Join-Path $Dist "$Name.zip"
if (Test-Path $Zip) { Remove-Item $Zip }
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Host "==> $Zip"

# ---- MSI (optional, WiX v3) ------------------------------------------------------------------------
if ((Get-Command candle.exe -ErrorAction SilentlyContinue) -and (Get-Command light.exe -ErrorAction SilentlyContinue)) {
    $Wxs = Join-Path $Work 'vizcraft.wxs'
    $MsiVersion = ($Version -replace '-.*$', '')
    @"
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="VizCraft" Language="1033" Version="$MsiVersion" Manufacturer="VizCraft" UpgradeCode="6C3F1A52-8E0B-4D7B-9C1E-2B7A5D9E4F10">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perUser" />
    <MajorUpgrade DowngradeErrorMessage="A newer VizCraft is installed." />
    <MediaTemplate EmbedCab="yes" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="LocalAppDataFolder">
        <Directory Id="Programs" Name="Programs">
          <Directory Id="INSTALLDIR" Name="VizCraft">
            <Component Id="App" Guid="*">
              <File Id="VizCraftExe" Source="$Stage\vizcraft.exe" KeyPath="no" />
              <File Id="ServerExe" Source="$Stage\vizcraft-server.exe" />
              <RegistryValue Root="HKCU" Key="Software\VizCraft" Name="installed" Type="integer" Value="1" KeyPath="yes" />
              <RemoveFolder Id="RemoveInstallDir" On="uninstall" />
            </Component>
          </Directory>
        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Component Id="Shortcut" Guid="*">
          <Shortcut Id="StartMenu" Name="VizCraft" Target="[INSTALLDIR]vizcraft.exe" WorkingDirectory="INSTALLDIR" />
          <RegistryValue Root="HKCU" Key="Software\VizCraft" Name="shortcut" Type="integer" Value="1" KeyPath="yes" />
        </Component>
      </Directory>
    </Directory>
    <Feature Id="Main" Title="VizCraft" Level="1">
      <ComponentRef Id="App" />
      <ComponentRef Id="Shortcut" />
    </Feature>
  </Product>
</Wix>
"@ | Set-Content -Encoding UTF8 $Wxs
    candle.exe -nologo -out (Join-Path $Work 'vizcraft.wixobj') $Wxs
    light.exe -nologo -sice:ICE38 -sice:ICE64 -sice:ICE91 -out (Join-Path $Dist "$Name.msi") (Join-Path $Work 'vizcraft.wixobj')
    Write-Host "==> $(Join-Path $Dist "$Name.msi")"
} else {
    Write-Host '(WiX not found: skipping the MSI)'
}
Get-ChildItem $Dist
