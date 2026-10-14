; Inno Setup script for the zeteme installer. Build with:
;   iscc /DAppVersion=0.2.0 installer\zeteme.iss
; It installs per user (no admin prompt), which also lets zeteme update
; itself by running a newer installer silently.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{B425FE8B-D338-4397-9659-3A93F28956CA}
AppName=zeteme
AppVersion={#AppVersion}
AppVerName=zeteme {#AppVersion}
AppPublisher=zeteme
AppPublisherURL=https://github.com/tacobellerontop-sudo/zeteme
AppUpdatesURL=https://github.com/tacobellerontop-sudo/zeteme/releases
DefaultDirName={autopf}\zeteme
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
UninstallDisplayIcon={app}\zeteme.exe
OutputDir=..\target\installer
OutputBaseFilename=zeteme-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\zeteme.ico

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\zeteme.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\zeteme"; Filename: "{app}\zeteme.exe"
Name: "{autodesktop}\zeteme"; Filename: "{app}\zeteme.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\zeteme.exe"; Description: "{cm:LaunchProgram,zeteme}"; Flags: nowait postinstall skipifsilent
; A silent install is an update started from zeteme itself, so bring it back.
Filename: "{app}\zeteme.exe"; Flags: nowait skipifnotsilent
