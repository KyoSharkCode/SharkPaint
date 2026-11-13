; Instalador de SharkPaint para Windows (Inno Setup 6).
;
; Lo compila la integración continua (.github/workflows/build.yml) a partir de la carpeta
; con el ejecutable y sus licencias:
;   iscc /DAppVersion=0.1.0 /DSourceDir=<carpeta instalada> /DOutputDir=<salida> SharkPaint.iss
;
; Se instala para el usuario (sin permisos de administrador) y asocia los .spaint con
; SharkPaint.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\SharkPaint"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
; Identificador fijo: con él, una versión nueva sustituye a la anterior.
AppId={{054B618B-73DB-4F70-8833-4668CEAD79F2}
AppName=SharkPaint
AppVersion={#AppVersion}
AppVerName=SharkPaint {#AppVersion}
AppPublisher=KyoSumiVT
AppCopyright=Copyright (C) 2026 KyoSumiVT
AppPublisherURL=https://kyosharkcode.github.io/SharkPaint/
AppSupportURL=https://github.com/KyoSharkCode/SharkPaint/issues
AppUpdatesURL=https://kyosharkcode.github.io/SharkPaint/
DefaultDirName={autopf}\SharkPaint
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=SharkPaint-{#AppVersion}-Setup
SetupIconFile=..\assets\icons\sharkpaint.ico
UninstallDisplayIcon={app}\SharkPaint.exe
UninstallDisplayName=SharkPaint
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Cierra SharkPaint si está abierto (al actualizar).
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "escritorio"; Description: "Crear un acceso directo en el escritorio"; GroupDescription: "Accesos directos:"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\SharkPaint"; Filename: "{app}\SharkPaint.exe"
Name: "{autodesktop}\SharkPaint"; Filename: "{app}\SharkPaint.exe"; Tasks: escritorio

[Registry]
; Proyectos .spaint: se abren con SharkPaint (doble clic) y llevan su icono.
Root: HKA; Subkey: "Software\Classes\.spaint"; ValueType: string; ValueName: ""; ValueData: "SharkPaint.Proyecto"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\SharkPaint.Proyecto"; ValueType: string; ValueName: ""; ValueData: "Proyecto de SharkPaint"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\SharkPaint.Proyecto\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\SharkPaint.exe,0"
Root: HKA; Subkey: "Software\Classes\SharkPaint.Proyecto\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\SharkPaint.exe"" ""%1"""

[Run]
Filename: "{app}\SharkPaint.exe"; Description: "Abrir SharkPaint"; Flags: nowait postinstall skipifsilent
