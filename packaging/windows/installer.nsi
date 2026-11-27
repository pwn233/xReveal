; The Windows installer (docs/spec/v1.md §6): per user, no admin, into
; %LOCALAPPDATA%\Programs\xReveal, with a Start-menu entry and an uninstaller.
; Built by scripts/package-windows.ps1: makensis /DVERSION=x.y.z /DEXE=path /DOUT=path installer.nsi

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; xReveal's in-app update (#163) runs this silently with /S /RELAUNCH: wait for the running
; xReveal to quit (its exe stays locked until then), install, and start the new version.
Var Relaunch

Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RELAUNCH" $R1
  ${IfNot} ${Errors}
    StrCpy $Relaunch "1"
    StrCpy $R2 0
    ${Do}
      ${IfNot} ${FileExists} "$INSTDIR\xreveal.exe"
        ${Break}
      ${EndIf}
      ClearErrors
      FileOpen $R3 "$INSTDIR\xreveal.exe" a
      ${IfNot} ${Errors}
        FileClose $R3
        ${Break}
      ${EndIf}
      Sleep 250
      IntOp $R2 $R2 + 1
    ${LoopUntil} $R2 >= 80
  ${EndIf}
FunctionEnd

Name "xReveal"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\xReveal"
InstallDirRegKey HKCU "Software\xReveal" "InstallDir"
SetCompressor /SOLID lzma

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "xReveal"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "xReveal installer"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

; The app icon (#130), for the installer and uninstaller too.
!define MUI_ICON "xreveal.ico"
!define MUI_UNICON "xreveal.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\xreveal.exe"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\xReveal"

Section "xReveal"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "/oname=xreveal.exe" "${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortCut "$SMPROGRAMS\xReveal.lnk" "$INSTDIR\xreveal.exe"
  WriteRegStr HKCU "Software\xReveal" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "xReveal"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "xReveal"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\xreveal.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${If} $Relaunch == "1"
    Exec '"$INSTDIR\xreveal.exe"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\xReveal.lnk"
  Delete "$INSTDIR\xreveal.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\xReveal"
  ; The Remembered setup (%LOCALAPPDATA%\xReveal) is left alone, so reinstalling keeps it.
SectionEnd
