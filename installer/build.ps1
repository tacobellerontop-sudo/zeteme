# Builds target\installer\zeteme-setup.exe from target\release\zeteme.exe,
# installing Inno Setup first if the machine doesn't have it.
$ErrorActionPreference = "Stop"

$version = (Select-String -Path "$PSScriptRoot\..\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup --yes --no-progress
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
}
& $iscc "/DAppVersion=$version" "$PSScriptRoot\zeteme.iss"
exit $LASTEXITCODE
