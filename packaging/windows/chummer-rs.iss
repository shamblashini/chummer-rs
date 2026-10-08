; Inno Setup script for the chummer-rs Windows installer.
; The release workflow builds it with:
;   iscc /DAppVersion=0.5.0 /DSourceDir=<package dir> /DOutputDir=<dir> /DOutputName=<name> packaging\windows\chummer-rs.iss
; Per-user install (no administrator rights) to %LOCALAPPDATA%\Programs\chummer-rs
; by default; the first page lets the user install for all users instead.
; The in-app updater runs the new installer with
;   /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RELAUNCH

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\chummer-rs"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif
#ifndef OutputName
  #define OutputName "chummer-rs-setup"
#endif

[Setup]
; Never change AppId: updates find the existing install through it.
AppId={{EACE9962-C255-4CA6-9377-C9A9322392D1}
AppName=chummer-rs
AppVersion={#AppVersion}
AppVerName=chummer-rs {#AppVersion}
AppPublisher=chummer-rs
AppPublisherURL=https://github.com/shamblashini/chummer-rs
AppSupportURL=https://github.com/shamblashini/chummer-rs/issues
AppUpdatesURL=https://github.com/shamblashini/chummer-rs/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\chummer-rs
DefaultGroupName=chummer-rs
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
SetupIconFile=..\chummer-rs.ico
UninstallDisplayIcon={app}\chummer-rs.exe
UninstallDisplayName=chummer-rs
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
; Chummer5a may own .chum5: only take it over when asked. chummer-rs is
; always offered in "Open with" for them.
Name: "chum5"; Description: "Open Chummer5a characters (.chum5, .chum5lz) with chummer-rs"; GroupDescription: "File associations:"; Flags: unchecked

[InstallDelete]
; Data files removed in a new version must not linger.
Type: filesandordirs; Name: "{app}\resources"
Type: filesandordirs; Name: "{app}\xsltproc"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\chummer-rs"; Filename: "{app}\chummer-rs.exe"; Comment: "Shadowrun 5th Edition character manager"
Name: "{autodesktop}\chummer-rs"; Filename: "{app}\chummer-rs.exe"; Tasks: desktopicon

[Registry]
; HKA: HKCU for a per-user install, HKLM for an all-users one.
; chummer-rs's own files.
Root: HKA; Subkey: "Software\Classes\.chumrs"; ValueType: string; ValueName: ""; ValueData: "chummer-rs.character"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.chummercampaign"; ValueType: string; ValueName: ""; ValueData: "chummer-rs.campaign"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\chummer-rs.character"; ValueType: string; ValueName: ""; ValueData: "chummer-rs character"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\chummer-rs.character\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\chummer-rs.exe,0"
Root: HKA; Subkey: "Software\Classes\chummer-rs.character\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chummer-rs.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\chummer-rs.campaign"; ValueType: string; ValueName: ""; ValueData: "chummer-rs campaign"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\chummer-rs.campaign\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\chummer-rs.exe,0"
Root: HKA; Subkey: "Software\Classes\chummer-rs.campaign\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chummer-rs.exe"" ""%1"""
; Chummer5a files: always in "Open with", the default only with the task.
Root: HKA; Subkey: "Software\Classes\chummer-rs.chum5"; ValueType: string; ValueName: ""; ValueData: "Chummer5a character"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\chummer-rs.chum5\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\chummer-rs.exe,0"
Root: HKA; Subkey: "Software\Classes\chummer-rs.chum5\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chummer-rs.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.chum5\OpenWithProgids"; ValueType: string; ValueName: "chummer-rs.chum5"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.chum5lz\OpenWithProgids"; ValueType: string; ValueName: "chummer-rs.chum5"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.chum5"; ValueType: string; ValueName: ""; ValueData: "chummer-rs.chum5"; Flags: uninsdeletevalue; Tasks: chum5
Root: HKA; Subkey: "Software\Classes\.chum5lz"; ValueType: string; ValueName: ""; ValueData: "chummer-rs.chum5"; Flags: uninsdeletevalue; Tasks: chum5
; Invite links: chummer-rs://join/...
Root: HKA; Subkey: "Software\Classes\chummer-rs"; ValueType: string; ValueName: ""; ValueData: "URL:chummer-rs invite link"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\chummer-rs"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\chummer-rs\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\chummer-rs.exe,0"
Root: HKA; Subkey: "Software\Classes\chummer-rs\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chummer-rs.exe"" ""%1"""

[Run]
Filename: "{app}\chummer-rs.exe"; Description: "{cm:LaunchProgram,chummer-rs}"; Flags: nowait postinstall skipifsilent
; The in-app updater passes /RELAUNCH to start the new version afterwards.
Filename: "{app}\chummer-rs.exe"; Flags: nowait; Check: Relaunch

[Code]
function Relaunch: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then
      Result := True;
end;
