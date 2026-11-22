; Inno Setup script for the Windows host.
; Built from CI with:
;   ISCC.exe /DMyAppVersion=x.y.z /DMyAppTag=vx.y.z /DSourceDir=... /DOutputDir=... eiviz.iss

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef MyAppBase
  #define MyAppBase MyAppVersion
#endif
#ifndef MyAppTag
  #define MyAppTag "v" + MyAppVersion
#endif
#ifndef SourceDir
  #define SourceDir "..\..\publish\win-x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif
#ifndef RepoRoot
  #define RepoRoot "..\.."
#endif
; An edition build for one event passes its own display name. Everything the
; installed copy is found by — AppId, install folder, executable name — stays
; "eiviz", so an edition replaces the normal install instead of adding a second
; one beside it.
#ifndef MyAppName
  #define MyAppName "eiviz"
#endif

#define MyAppPublisher "Mikansei Laboratory"
#define MyAppPublisherDir "MikanseiLaboratory"
#define MyAppURL "https://github.com/MikanseiLaboratory/eiviz"
#define MyAppExeName "Eiviz.Host.exe"
#define MyAppDirName "eiviz"
#define MyAppIcon RepoRoot + "\docs\src\assets\branding\app-icon\windows\eiviz.ico"

[Setup]
AppId={{8F3C2A71-6B4E-4D19-9A2C-71E104A0B164}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppPublisherDir}\{#MyAppDirName}
DefaultGroupName={#MyAppPublisherDir}
DisableProgramGroupPage=yes
LicenseFile={#SourceDir}\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=eiviz-{#MyAppTag}-win-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile={#MyAppIcon}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
ChangesAssociations=yes
UninstallDisplayIcon={app}\{#MyAppExeName}
VersionInfoVersion={#MyAppBase}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

; Shortcut icons come from the installed executables, which embed the app icon
; through ApplicationIcon. They must not point into the build tree: that path
; does not exist on the user's machine, and Windows then falls back to the
; generic page icon for the running app in the taskbar as well.
[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{group}\{#MyAppName} Remote"; Filename: "{app}\Eiviz.Remote.exe"; IconFilename: "{app}\Eiviz.Remote.exe"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; IconFilename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName} Remote"; Filename: "{app}\Eiviz.Remote.exe"; Tasks: desktopicon; IconFilename: "{app}\Eiviz.Remote.exe"

[Registry]
Root: HKCR; Subkey: ".eivz"; ValueType: string; ValueName: ""; ValueData: "eiviz.session"; Flags: uninsdeletevalue
Root: HKCR; Subkey: ".eivzx"; ValueType: string; ValueName: ""; ValueData: "eiviz.session"; Flags: uninsdeletevalue
Root: HKCR; Subkey: "eiviz.session"; ValueType: string; ValueName: ""; ValueData: "eiviz Session"; Flags: uninsdeletekey
Root: HKCR; Subkey: "eiviz.session\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName},0"
Root: HKCR; Subkey: "eiviz.session\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
