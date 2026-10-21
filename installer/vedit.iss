; ============================================================
;  VEDIT — Inno Setup installer script
;
;  Produces: installer/Output/VEDIT-Setup-<version>.exe
;  Installs: vedit.exe + bundled ffmpeg/ffprobe + Start Menu / Desktop icons
;
;  Usage (from the VEDIT root, after `cargo build --release` and
;  placing ffmpeg.exe + ffprobe.exe in dist/VEDIT/ffmpeg/):
;
;    ISCC installer\vedit.iss /DAppVersion=0.2.1
;
; ============================================================

#define MyAppName "VEDIT"
#ifndef AppVersion
#define AppVersion "0.2.1"
#endif
#define MyAppPublisher "VEDIT contributors"
#define MyAppExeName "vedit.exe"

[Setup]
AppId={{7C1E5E42-8D2C-4F5A-9E7B-VEDIT0000001}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\VEDIT
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=Output
OutputBaseFilename=VEDIT-Setup-{#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
MinVersion=10.0
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addpath"; Description: "Add VEDIT's ffmpeg to your PATH (lets other tools use it)"; GroupDescription: "Options:"; Flags: unchecked

[Files]
; Main editor binary
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; Bundled ffmpeg engine (all formats & codecs)
Source: "..\dist\VEDIT\ffmpeg\ffmpeg.exe"; DestDir: "{app}\ffmpeg"; Flags: ignoreversion
Source: "..\dist\VEDIT\ffmpeg\ffprobe.exe"; DestDir: "{app}\ffmpeg"; Flags: ignoreversion
; Docs
Source: "..\README.md"; DestDir: "{app}"; Flags: skipifsourcedoesntexist
Source: "..\LICENSE"; DestDir: "{app}"; Flags: skipifsourcedoesntexist

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Dirs]
Name: "{app}\ffmpeg"; Permissions: users-modify

[Code]
function FileExistsInDist(Param: string): Boolean;
begin
  Result := True; // presence is enforced by the build script; missing files abort setup compile
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    // nothing extra needed: vedit auto-discovers .\ffmpeg\ffmpeg.exe next to the exe
  end;
end;

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; remove nothing user-created; binaries are removed automatically
