; Inno Setup script for tidal-rs. Build with:
;   iscc /DAppVersion=0.1.0 installer\tidal-rs.iss
; Per-user install: no administrator rights needed.

#ifndef AppVersion
	#define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1B7C1E-3A0D-4C59-9B67-2D1E7A4F5C31}
AppName=tidal-rs
AppVersion={#AppVersion}
AppPublisher=tidal-rs
DefaultDirName={localappdata}\Programs\tidal-rs
DefaultGroupName=tidal-rs
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
OutputDir=..\target\installer
OutputBaseFilename=tidal-rs-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\tidal-rs.exe
UninstallDisplayName=tidal-rs
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\tidal-rs.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\tidal-rs"; Filename: "{app}\tidal-rs.exe"; AppUserModelID: "TidalRs.Player"
Name: "{autodesktop}\tidal-rs"; Filename: "{app}\tidal-rs.exe"; AppUserModelID: "TidalRs.Player"; Tasks: desktopicon

[Run]
Filename: "{app}\tidal-rs.exe"; Description: "{cm:LaunchProgram,tidal-rs}"; Flags: nowait postinstall skipifsilent
