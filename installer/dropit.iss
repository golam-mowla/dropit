; Inno Setup Script for Drop It
; Created by Golam Mowla (https://github.com/golam-mowla)

#define MyAppName "Drop It"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Golam Mowla"
#define MyAppURL "https://github.com/golam-mowla/dropit"
#define MyAppExeName "dropit.exe"

[Setup]
AppId={{8B1D4A32-9F1C-4B9A-8E4D-7C5F3A1E2B9D}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\DropIt
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\target\installer
OutputBaseFilename=dropit-setup-x64
SetupIconFile=..\assets\dropit.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "startup"; Description: "Start Drop It automatically on Windows boot (minimized to system tray)"; GroupDescription: "Windows Integration:"
Name: "contextmenu"; Description: "Add 'Send with Drop It' to Windows Explorer right-click context menu"; GroupDescription: "Windows Integration:"

#ifndef SourceExe
#define SourceExe "..\target\release\" + MyAppExeName
#endif

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "..\assets\icon.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: isreadme ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Run on Windows Startup (HKCU)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "DropIt"; ValueData: """{app}\{#MyAppExeName}"" --tray"; Tasks: startup; Flags: uninsdeletevalue

; Windows Explorer Context Menu for Files
Root: HKCU; Subkey: "Software\Classes\*\shell\DropIt"; ValueType: string; ValueName: ""; ValueData: "Send with Drop It"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\DropIt"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\*\shell\DropIt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" send ""%1"""; Tasks: contextmenu

; Windows Explorer Context Menu for Folders / Directories
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DropIt"; ValueType: string; ValueName: ""; ValueData: "Send with Drop It"; Tasks: contextmenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DropIt"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"",0"; Tasks: contextmenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\DropIt\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" send ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
