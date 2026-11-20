; NSIS-Installer für LENZFLIX Desktop (Benutzerinstallation, kein Administrator nötig).
; Aufruf durch `cargo xtask package`, mit -DVERSION -DVIVERSION -DSTAGE -DOUTFILE -DARCH.
;   VERSION   z. B. 0.1.0          VIVERSION  z. B. 0.1.0.0
;   STAGE     Ordner mit lenzflix.exe, libmpv-2.dll, lenzflix.ico, LICENSES\
;   OUTFILE   Zieldatei            ARCH       arm64 oder x64 (nur Anzeige)
; Stiller Modus: LENZFLIX_..._setup.exe /S [/D=<Ordner>] (/D muss das letzte Argument sein).

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!define APPNAME "LENZFLIX"
!define EXENAME "lenzflix.exe"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\LENZFLIX"

!include "MUI2.nsh"

Name "${APPNAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "Software\${APPNAME}" "InstallDir"

VIProductVersion "${VIVERSION}"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} Installer (${ARCH})"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!define MUI_ICON "${STAGE}\lenzflix.ico"
!define MUI_UNICON "${STAGE}\lenzflix.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXENAME}"
!define MUI_FINISHPAGE_RUN_TEXT "LENZFLIX starten"

!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSES\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "German"

Section "Install"
  SetOutPath "$INSTDIR"
  File "${STAGE}\${EXENAME}"
  File "${STAGE}\libmpv-2.dll"
  File "${STAGE}\lenzflix.ico"
  SetOutPath "$INSTDIR\LICENSES"
  File /r "${STAGE}\LICENSES\*.*"
  SetOutPath "$INSTDIR"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\lenzflix.ico" 0

  WriteRegStr HKCU "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\lenzflix.ico"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

; Nutzerdaten (Konfiguration, Cache, Downloads unter %APPDATA% / %LOCALAPPDATA%) bleiben bewusst erhalten.
Section "Uninstall"
  Delete "$INSTDIR\${EXENAME}"
  Delete "$INSTDIR\libmpv-2.dll"
  Delete "$INSTDIR\lenzflix.ico"
  RMDir /r "$INSTDIR\LICENSES"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\${APPNAME}"
SectionEnd
