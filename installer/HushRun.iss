; HushRun installer (Inno Setup 6).
; Built by the Release workflow: ISCC /DAppVersion=<x.y.z> /DTagName=<vX.Y.Z> installer\HushRun.iss
; Produces dist\HushRun-Setup-<TagName>-x64.exe — a per-user install
; (no admin needed). The portable download is the same hushrun.exe,
; published next to this installer on every release.
; User data (sessions + settings in %LOCALAPPDATA%\HushRun) lives
; outside the install folder and is kept on uninstall.

#ifndef AppVersion
  #define AppVersion "0.4.0"
#endif
#ifndef TagName
  #define TagName "v0.4.0"
#endif

#define AppName "HushRun"

[Setup]
AppId={{0F06E875-8F83-4C2D-A76E-94EA73CD733E}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Sean Shaffer
AppPublisherURL=https://github.com/SeanShaffer88/HushRun
AppSupportURL=https://github.com/SeanShaffer88/HushRun
DefaultDirName={autopf}\HushRun
DefaultGroupName=HushRun
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=HushRun-Setup-{#TagName}-x64
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\hushrun.exe
UninstallDisplayName=HushRun
WizardStyle=modern

[Files]
Source: "..\target\release\hushrun.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\HushRun"; Filename: "{app}\hushrun.exe"
Name: "{group}\Uninstall HushRun"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\hushrun.exe"; Description: "Launch HushRun now"; Flags: nowait postinstall skipifsilent
