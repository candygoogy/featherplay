; featherplay's per-user installer (Inno Setup 6.7). Build it with build-installer.sh, which
; passes the defines below; see that script for what each one is.
;
; - Installs for the current user only, with no admin prompt, to
;   %LOCALAPPDATA%\Programs\featherplay. An upgrade installs over the old version in place
;   (same AppId) and keeps the settings and sign-in.
; - The Start menu shortcut carries the app's AppUserModelID, so the media overlay, the
;   taskbar and notifications name featherplay. The app checks the same shortcut at every
;   start (crates/platform/src/common/identity.rs: AUMID, SHORTCUT_FILE); keep the two equal.
; - Uninstalling removes everything featherplay keeps: the program folder, its settings
;   (%APPDATA%\featherplay), its cache, sidecars and sign-in browser profile
;   (%LOCALAPPDATA%\featherplay), its secrets in Windows Credential Manager, the registry
;   keys of its AppUserModelID, and the shortcuts. Only exact names that are featherplay's
;   own; nothing is matched by wildcard outside those folders.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z (build-installer.sh reads it from Cargo).
#endif
#ifndef ExeFile
  #error Pass /DExeFile=<path to the release featherplay.exe>.
#endif
; The folder where apps/featherplay/build.rs put featherplay.ico (the exe's icon) and the
; wizard images installer-{small,panel}{,-light}.png: its OUT_DIR.
#ifndef ArtDir
  #error Pass /DArtDir=<the featherplay build script's OUT_DIR>.
#endif
; The Visual C++ runtime the exe links (through Skia), for Arch: the build tools'
; Microsoft.VC14x.CRT folder (build-installer.sh finds it). Installed beside the exe, so
; featherplay starts on a Windows without the VC++ Redistributable.
#ifndef CrtDir
  #error Pass /DCrtDir=<the Visual C++ runtime's Microsoft.VC14x.CRT folder for Arch>.
#endif
#ifndef OutputDir
  #define OutputDir "..\..\target\installer"
#endif
; The service part of the credential names (crates/store/src/secrets.rs: SERVICE). Only the
; installer test changes it, so that a test uninstall can't touch real credentials.
#ifndef CredService
  #define CredService "featherplay"
#endif
; The exe's architecture: x64 (the default) or arm64. Both setups share the AppId, so either
; one installs over the other in place. The x64 setup also installs on ARM64 Windows 11,
; which runs it under emulation: 0.1.0 users there update through it, and its asset name
; stays featherplay-setup-<version>.exe. The ARM64 setup installs only on ARM64.
#ifndef Arch
  #define Arch "x64"
#endif
#if Arch == "x64"
  #define ArchAllowed "x64compatible"
  #define SetupSuffix ""
#elif Arch == "arm64"
  #define ArchAllowed "arm64"
  #define SetupSuffix "-arm64"
#else
  #error Arch must be x64 or arm64.
#endif

#define AppName "featherplay"
; crates/platform/src/common/identity.rs: AUMID.
#define AppUserModelId "featherplay.featherplay"
; apps/featherplay/src/endsession.rs: CLASS and TITLE.
#define EndSessionClass "featherplay-endsession"
#define EndSessionTitle "featherplay"

[Setup]
; Never change the AppId: upgrades find the installed copy by it.
AppId={{F2D85B2D-D1BB-4404-AA87-B6D35D0F5283}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=candygoogy
AppPublisherURL=https://github.com/candygoogy/featherplay
AppSupportURL=https://github.com/candygoogy/featherplay/issues
AppUpdatesURL=https://github.com/candygoogy/featherplay/releases
AppCopyright=Copyright (c) candygoogy. MIT licence.
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} setup

; Per user, no admin prompt, and no way to ask for one.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchAllowed}
MinVersion=10.0

; A short wizard: the shortcuts choice, the progress, and the finish page with "Launch".
DisableWelcomePage=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
ShowLanguageDialog=no
WizardStyle=modern dynamic
SetupIconFile={#ArtDir}\featherplay.ico
; The light logo (a crimson ring round the play) on a light wizard, the dark one on dark.
WizardSmallImageFile={#ArtDir}\installer-small-light.png
WizardSmallImageFileDynamicDark={#ArtDir}\installer-small.png
WizardImageFile={#ArtDir}\installer-panel-light.png
WizardImageFileDynamicDark={#ArtDir}\installer-panel.png
WizardImageStretch=no

; A running featherplay is closed by the Restart Manager, which sends the end-session
; messages endsession.rs answers by saving and quitting. It isn't restarted that way; the
; finish page's "Launch featherplay" does it, or, for an update featherplay started itself
; (/RELAUNCH), the second [Run] entry.
CloseApplications=yes
RestartApplications=no

UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\featherplay.exe

OutputDir={#OutputDir}
OutputBaseFilename=featherplay-setup-{#AppVersion}{#SetupSuffix}
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExeFile}"; DestDir: "{app}"; DestName: "featherplay.exe"; Flags: ignoreversion
; The Visual C++ runtime, app-local: Windows looks in the exe's folder first.
Source: "{#CrtDir}\msvcp140.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#CrtDir}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion
#if FileExists(CrtDir + "\vcruntime140_1.dll")
Source: "{#CrtDir}\vcruntime140_1.dll"; DestDir: "{app}"; Flags: ignoreversion
#endif
; featherplay's licence and the third-party notices, which the BSD, MIT and IJG licences of
; code linked into the exe ask to ship with it. As .txt, so a double click opens them.
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\THIRD_PARTY.md"; DestDir: "{app}"; DestName: "THIRD_PARTY.txt"; Flags: ignoreversion

[Icons]
; Directly in the Start menu's Programs folder, where the app itself looks for it
; (identity.rs: SHORTCUT_FILE), so it doesn't make a second one.
Name: "{userprograms}\{#AppName}"; Filename: "{app}\featherplay.exe"; WorkingDir: "{app}"; Comment: "featherplay music player"; AppUserModelID: "{#AppUserModelId}"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\featherplay.exe"; WorkingDir: "{app}"; Comment: "featherplay music player"; AppUserModelID: "{#AppUserModelId}"; Tasks: desktopicon

[Run]
Filename: "{app}\featherplay.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; An update featherplay downloaded and started itself, silently, with /RELAUNCH
; (apps/featherplay/src/updates/install.rs: SETUP_ARGS): the new version starts once its
; files are in place. A plain silent install (/VERYSILENT alone) still starts nothing.
Filename: "{app}\featherplay.exe"; Flags: nowait; Check: Relaunch

[UninstallDelete]
; The app never writes into {app}; Inno removes the files it installed and then the folder
; if it's empty. The data folders are removed in [Code] (DeleteDataFolder), with retries.
Type: dirifempty; Name: "{app}"

[Code]
const
  // The secrets' names (crates/store/src/secrets.rs: SecretKey::name). A long value is
  // split into chunks named <name>#1, <name>#2 and so on.
  SecretNames = 'yt-cookies,lastfm-session,lastfm-api-key,lastfm-api-secret,listenbrainz-token,local-api-token';
  // More chunks than any cookie header needs (2560 bytes each).
  MaxChunks = 64;
  CRED_TYPE_GENERIC = 1;

  SYNCHRONIZE = $00100000;
  PROCESS_TERMINATE = $0001;
  PROCESS_QUERY_LIMITED_INFORMATION = $1000;
  WAIT_OBJECT_0 = 0;
  WM_QUERYENDSESSION = $0011;
  WM_ENDSESSION = $0016;
  ENDSESSION_CLOSEAPP = 1;
  SMTO_ABORTIFHUNG = 2;

type
  TProcList = record
    Pids: array of DWORD;
    Handles: array of THandle;
  end;

function CredDeleteW(TargetName: String; CredType: DWORD; Flags: DWORD): BOOL;
  external 'CredDeleteW@advapi32.dll stdcall';
function OpenProcess(Access: DWORD; Inherit: BOOL; Pid: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function QueryFullProcessImageNameW(Process: THandle; Flags: DWORD; Name: String; var Size: DWORD): BOOL;
  external 'QueryFullProcessImageNameW@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function TerminateProcess(Process: THandle; Code: UINT): BOOL;
  external 'TerminateProcess@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';
function FindWindowExW(Parent, After: HWND; ClassName, WindowName: String): HWND;
  external 'FindWindowExW@user32.dll stdcall';
function GetWindowThreadProcessId(Wnd: HWND; var Pid: DWORD): DWORD;
  external 'GetWindowThreadProcessId@user32.dll stdcall';
function SendMessageTimeoutW(Wnd: HWND; Msg: UINT; WParam: Longint; LParam: Longint; Flags: UINT; Timeout: UINT; var Answer: DWORD): Longint;
  external 'SendMessageTimeoutW@user32.dll stdcall';

// The full path of the exe a process runs, read through its handle (so the answer is about
// that very process, even if its ID is reused later).
function ImagePath(Process: THandle): String;
var
  Size: DWORD;
begin
  Result := '';
  Size := 1024;
  SetLength(Result, Size);
  if QueryFullProcessImageNameW(Process, 0, Result, Size) then
    SetLength(Result, Size)
  else
    Result := '';
end;

function Listed(const List: TProcList; Pid: DWORD): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 0 to GetArrayLength(List.Pids) - 1 do
    if List.Pids[I] = Pid then begin
      Result := True;
      Exit;
    end;
end;

// Adds the process if it runs exactly ExePath, keeping a handle that pins it.
procedure AddIfRuns(var List: TProcList; Pid: DWORD; const ExePath: String);
var
  H: THandle;
  N: Integer;
begin
  if (Pid = 0) or Listed(List, Pid) then
    Exit;
  H := OpenProcess(SYNCHRONIZE or PROCESS_TERMINATE or PROCESS_QUERY_LIMITED_INFORMATION, False, Pid);
  if H = 0 then
    Exit;
  if CompareText(ImagePath(H), ExePath) = 0 then begin
    N := GetArrayLength(List.Pids);
    SetArrayLength(List.Pids, N + 1);
    SetArrayLength(List.Handles, N + 1);
    List.Pids[N] := Pid;
    List.Handles[N] := H;
  end else
    CloseHandle(H);
end;

// The processes running exactly ExePath: WMI lists the candidates by file name, then each
// one's path is checked through its own handle.
procedure FindByPath(var List: TProcList; const ExePath: String);
var
  Wmi, Procs, P: Variant;
  I: Integer;
begin
  try
    Wmi := CreateOleObject('WbemScripting.SWbemLocator');
    Procs := Wmi.ConnectServer('.', 'root\CIMV2').ExecQuery(
      'SELECT ProcessId FROM Win32_Process WHERE Name = ''' + ExtractFileName(ExePath) + '''');
    for I := 0 to Procs.Count - 1 do begin
      P := Procs.ItemIndex(I);
      AddIfRuns(List, P.ProcessId, ExePath);
    end;
  except
    Log('Listing processes through WMI failed: ' + GetExceptionMessage);
  end;
end;

procedure CloseHandles(var List: TProcList);
var
  I: Integer;
begin
  for I := 0 to GetArrayLength(List.Handles) - 1 do
    CloseHandle(List.Handles[I]);
  SetArrayLength(List.Pids, 0);
  SetArrayLength(List.Handles, 0);
end;

// Waits up to Milliseconds for every listed process to exit, then ends the ones left.
procedure WaitThenEnd(var List: TProcList; Milliseconds: Integer);
var
  I, Waited: Integer;
begin
  Waited := 0;
  for I := 0 to GetArrayLength(List.Handles) - 1 do begin
    while (WaitForSingleObject(List.Handles[I], 0) <> WAIT_OBJECT_0) and (Waited < Milliseconds) do begin
      Sleep(100);
      Waited := Waited + 100;
    end;
    if WaitForSingleObject(List.Handles[I], 0) <> WAIT_OBJECT_0 then begin
      Log(Format('Process %d did not exit; ending it.', [List.Pids[I]]));
      TerminateProcess(List.Handles[I], 1);
      WaitForSingleObject(List.Handles[I], 5000);
    end else
      Log(Format('Process %d exited.', [List.Pids[I]]));
  end;
end;

// Closes the installed featherplay if it's running: asks it the way Windows does at
// sign-out (it saves its session and quits), then ends it if it hasn't gone in 10 seconds.
// A featherplay started from any other folder is left alone. Then its sidecars, if any
// outlived it.
procedure CloseFeatherplay();
var
  List, Sidecars: TProcList;
  Exe: String;
  Wnd: HWND;
  Pid, Answer: DWORD;
begin
  Exe := ExpandConstant('{app}\featherplay.exe');
  FindByPath(List, Exe);
  // Its end-session windows find it even if WMI couldn't.
  Wnd := FindWindowExW(0, 0, '{#EndSessionClass}', '{#EndSessionTitle}');
  while Wnd <> 0 do begin
    Pid := 0;
    GetWindowThreadProcessId(Wnd, Pid);
    AddIfRuns(List, Pid, Exe);
    Wnd := FindWindowExW(0, Wnd, '{#EndSessionClass}', '{#EndSessionTitle}');
  end;

  if GetArrayLength(List.Pids) > 0 then begin
    Log(Format('featherplay is running (%d processes); asking it to close.', [GetArrayLength(List.Pids)]));
    Wnd := FindWindowExW(0, 0, '{#EndSessionClass}', '{#EndSessionTitle}');
    while Wnd <> 0 do begin
      Pid := 0;
      GetWindowThreadProcessId(Wnd, Pid);
      if Listed(List, Pid) then begin
        SendMessageTimeoutW(Wnd, WM_QUERYENDSESSION, 0, ENDSESSION_CLOSEAPP, SMTO_ABORTIFHUNG, 5000, Answer);
        SendMessageTimeoutW(Wnd, WM_ENDSESSION, 1, ENDSESSION_CLOSEAPP, SMTO_ABORTIFHUNG, 8000, Answer);
      end;
      Wnd := FindWindowExW(0, Wnd, '{#EndSessionClass}', '{#EndSessionTitle}');
    end;
    WaitThenEnd(List, 10000);
  end;
  CloseHandles(List);

  // yt-dlp and deno from featherplay's own sidecars folder.
  FindByPath(Sidecars, ExpandConstant('{localappdata}\featherplay\sidecars\yt-dlp.exe'));
  FindByPath(Sidecars, ExpandConstant('{localappdata}\featherplay\sidecars\deno.exe'));
  if GetArrayLength(Sidecars.Pids) > 0 then
    WaitThenEnd(Sidecars, 0);
  CloseHandles(Sidecars);
end;

// featherplay's secrets in Credential Manager: <name>.<service> and its chunks
// <name>#<n>.<service>, by exact name.
procedure DeleteCredentials();
var
  Names: TStringList;
  I, Chunk: Integer;
  Target: String;
begin
  Names := TStringList.Create;
  try
    Names.CommaText := SecretNames;
    for I := 0 to Names.Count - 1 do
      for Chunk := 0 to MaxChunks - 1 do begin
        if Chunk = 0 then
          Target := Names[I] + '.{#CredService}'
        else
          Target := Names[I] + '#' + IntToStr(Chunk) + '.{#CredService}';
        if CredDeleteW(Target, CRED_TYPE_GENERIC, 0) then
          Log('Deleted credential ' + Target);
      end;
  finally
    Names.Free;
  end;
end;

// A data folder, retried for a few seconds: the sign-in browser's processes can hold
// files in it briefly after featherplay has exited.
procedure DeleteDataFolder(const Dir: String);
var
  Attempt: Integer;
begin
  for Attempt := 1 to 10 do begin
    if not DirExists(Dir) then
      Exit;
    DelTree(Dir, True, True, True);
    if not DirExists(Dir) then begin
      Log('Removed ' + Dir);
      Exit;
    end;
    Sleep(500);
  end;
  Log('Could not remove all of ' + Dir);
end;

procedure DeleteRegistryKeys();
begin
  // Written by the app at start (identity.rs: registry_key).
  RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Classes\AppUserModelId\{#AppUserModelId}');
  // Windows' notification settings for featherplay's AppUserModelID.
  RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{#AppUserModelId}');
end;

// featherplay started this setup to update itself: silent, with /RELAUNCH on the command
// line (apps/featherplay/src/updates/install.rs: SETUP_ARGS).
function Relaunch(): Boolean;
var
  I: Integer;
begin
  Result := False;
  if not WizardSilent then
    Exit;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then begin
      Result := True;
      Exit;
    end;
end;

// For an update featherplay started, featherplay quits on its own right after; this waits
// for it (and ends it if it hasn't gone in 10 seconds), so its exe isn't in use when the new
// one is copied. Other installs leave it to the Restart Manager, as before.
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  if Relaunch() then begin
    Log('Update started by featherplay; waiting for it to close.');
    CloseFeatherplay();
  end;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  case CurUninstallStep of
    // Before any file is removed.
    usUninstall:
      CloseFeatherplay();
    usPostUninstall:
      begin
        DeleteDataFolder(ExpandConstant('{userappdata}\featherplay'));
        DeleteDataFolder(ExpandConstant('{localappdata}\featherplay'));
        DeleteCredentials();
        DeleteRegistryKeys();
      end;
  end;
end;
