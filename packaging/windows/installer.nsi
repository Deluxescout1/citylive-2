; Citylive 2 per-user installer (NSIS, Unicode).
;
; Build:  bin/build-installer.sh   (passes the defines below)
;   makensis -DVERSION=0.2.0 -DSRCDIR=dist/windows -DICON=assets/windows/citylive.ico \
;            -DLICENSE=LICENSE -DOUTFILE=dist/Citylive2-Setup-0.2.0.exe packaging/windows/installer.nsi
;
; Installs to %LOCALAPPDATA%\Programs\Citylive 2 without administrator rights.
; Command line: /S silent, /D=dir (last), /UPDATE (silent update by the program itself:
; keeps the user's start-with-Windows choice and starts the new version),
; /NOAUTOSTART (silent: do not register start with Windows), /NOLAUNCH (silent: do not start it).

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!ifndef VERSION
  !error "pass -DVERSION=x.y.z"
!endif
!ifndef SRCDIR
  !define SRCDIR "dist\windows"
!endif
!ifndef OUTFILE
  !define OUTFILE "Citylive2-Setup-${VERSION}.exe"
!endif

!define APP "Citylive 2"
!define PUBLISHER "Deluxescout1"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Citylive2"
!define RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUNVALUE "Citylive 2"
!define OLDEXE "$LOCALAPPDATA\Programs\CityLive\CityLive.exe"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINSTKEY}" "InstallLocation"
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP} setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 ${PUBLISHER}"

!include MUI2.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include WordFunc.nsh
!include nsDialogs.nsh

!insertmacro GetParameters
!insertmacro GetOptions
!insertmacro GetSize
!insertmacro WordFind

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING

Var Dialog
Var AutoStartBox
Var AutoStart
Var OldBox
Var StopOld
Var OldFound
Var IsUpdate
Var NoLaunch
Var PurgeBox
Var Purge

; ------------------------------------------------------------------ pages
!insertmacro MUI_PAGE_WELCOME
!ifdef LICENSE
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif
!insertmacro MUI_PAGE_DIRECTORY
Page custom OptionsPage OptionsLeave
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP} now"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchApp
!define MUI_FINISHPAGE_LINK "Citylive 2 on GitHub"
!define MUI_FINISHPAGE_LINK_LOCATION "https://github.com/Deluxescout1/citylive-2"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
UninstPage custom un.OptionsPage un.OptionsLeave
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ------------------------------------------------------------------ helpers
; Stop programs of this install (by path, so another program with the same
; image name, such as the old CityLive, is not touched).
Function StopOurs
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-Process -Name citylive,citylive-settings -ErrorAction SilentlyContinue | Where-Object { $$_.Path -like '$INSTDIR\*' } | Stop-Process -Force"`
  Pop $0
  Sleep 800
FunctionEnd

Function StopOldCityLive
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-Process -Name CityLive -ErrorAction SilentlyContinue | Where-Object { $$_.Path -like '*\Programs\CityLive\*' } | Stop-Process -Force"`
  Pop $0
  ; Turn off the old program's start with Windows: every Run value that points into
  ; its folder. The program itself is left installed.
  StrCpy $1 0
  ${Do}
    EnumRegValue $2 HKCU "${RUNKEY}" $1
    ${If} $2 == ""
      ${Break}
    ${EndIf}
    ReadRegStr $3 HKCU "${RUNKEY}" "$2"
    ClearErrors
    ${WordFind} "$3" "\Programs\CityLive\CityLive.exe" "E+1{" $4
    ${If} ${Errors}
      IntOp $1 $1 + 1
    ${Else}
      DeleteRegValue HKCU "${RUNKEY}" "$2"
      ; the list shifted: look at the same index again
    ${EndIf}
  ${Loop}
FunctionEnd

Function .onInit
  ${GetParameters} $R0
  StrCpy $IsUpdate 0
  StrCpy $NoLaunch 0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $IsUpdate 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/NOLAUNCH" $R1
  ${IfNot} ${Errors}
    StrCpy $NoLaunch 1
  ${EndIf}
  ; Start with Windows: on for a fresh install, unchanged for an update.
  StrCpy $AutoStart 1
  ClearErrors
  ${GetOptions} $R0 "/NOAUTOSTART" $R1
  ${IfNot} ${Errors}
    StrCpy $AutoStart 0
  ${EndIf}
  ${If} $IsUpdate == 1
    ReadRegStr $R2 HKCU "${RUNKEY}" "${RUNVALUE}"
    ${If} $R2 == ""
      StrCpy $AutoStart 0
    ${Else}
      StrCpy $AutoStart 1
    ${EndIf}
  ${EndIf}
  ; The old Electron CityLive?
  StrCpy $OldFound 0
  IfFileExists "${OLDEXE}" 0 +2
    StrCpy $OldFound 1
  StrCpy $StopOld 0
FunctionEnd

; ------------------------------------------------------------------ options page
Function OptionsPage
  ${If} ${Silent}
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "Options" "Choose how ${APP} starts."
  nsDialogs::Create 1018
  Pop $Dialog
  ${If} $Dialog == error
    Abort
  ${EndIf}
  ${NSD_CreateCheckbox} 0 8u 100% 12u "Start ${APP} when I sign in to Windows"
  Pop $AutoStartBox
  ${If} $AutoStart == 1
    ${NSD_Check} $AutoStartBox
  ${EndIf}
  ${NSD_CreateLabel} 12u 22u 95% 24u "The wallpaper runs quietly in the background and pauses while a full-screen app is in front. You can change this later in the settings window or in Task Manager, Startup apps."
  Pop $0
  ${If} $OldFound == 1
    ${NSD_CreateCheckbox} 0 58u 100% 12u "Stop the old CityLive and turn off its start with Windows"
    Pop $OldBox
    ${NSD_Check} $OldBox
    ${NSD_CreateLabel} 12u 72u 95% 24u "The old CityLive was found on this computer. Two wallpapers cannot share the desktop. It is not uninstalled or deleted; you can remove it later from Settings, Apps."
    Pop $0
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function OptionsLeave
  ${NSD_GetState} $AutoStartBox $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $AutoStart 1
  ${Else}
    StrCpy $AutoStart 0
  ${EndIf}
  ${If} $OldFound == 1
    ${NSD_GetState} $OldBox $0
    ${If} $0 == ${BST_CHECKED}
      StrCpy $StopOld 1
    ${Else}
      StrCpy $StopOld 0
    ${EndIf}
  ${EndIf}
FunctionEnd

; ------------------------------------------------------------------ install
Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  Call StopOurs
  ; Silent installs (including updates) stop the old CityLive only when asked to
  ; by /STOPOLD; the interactive page asks the user.
  ${If} ${Silent}
    ClearErrors
    ${GetParameters} $R0
    ${GetOptions} $R0 "/STOPOLD" $R1
    ${IfNot} ${Errors}
      StrCpy $StopOld 1
    ${EndIf}
  ${EndIf}
  ${If} $StopOld == 1
    Call StopOldCityLive
  ${EndIf}

  File "/oname=citylive.exe" "${SRCDIR}\citylive.exe"
  File "/oname=citylive-settings.exe" "${SRCDIR}\citylive-settings.exe"
  !ifdef LICENSE
    File "/oname=LICENSE.txt" "${LICENSE}"
  !endif

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Start menu
  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\citylive.exe" "" "$INSTDIR\citylive.exe" 0
  CreateShortcut "$SMPROGRAMS\${APP}\${APP} Settings.lnk" "$INSTDIR\citylive-settings.exe" "" "$INSTDIR\citylive-settings.exe" 0

  ; Start with Windows (the same value the program's own checkbox writes)
  ${If} $AutoStart == 1
    WriteRegStr HKCU "${RUNKEY}" "${RUNVALUE}" '"$INSTDIR\citylive.exe"'
  ${Else}
    DeleteRegValue HKCU "${RUNKEY}" "${RUNVALUE}"
  ${EndIf}

  ; Add/Remove Programs
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\citylive.exe"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTKEY}" "URLInfoAbout" "https://github.com/Deluxescout1/citylive-2"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"

  ; Silent installs have no finish page: start it here.
  ${If} ${Silent}
  ${AndIf} $NoLaunch == 0
    Exec '"$INSTDIR\citylive.exe"'
  ${EndIf}
SectionEnd

Function LaunchApp
  Exec '"$INSTDIR\citylive.exe"'
FunctionEnd

; ------------------------------------------------------------------ uninstall
Function un.OptionsPage
  ${If} ${Silent}
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "Your worlds" "Choose what to keep."
  nsDialogs::Create 1018
  Pop $Dialog
  ${NSD_CreateLabel} 0 4u 100% 36u "Your saved cities and settings are kept by default, so installing ${APP} again brings the same city back."
  Pop $0
  ${NSD_CreateCheckbox} 0 46u 100% 12u "Also delete my saved cities, settings and caches"
  Pop $PurgeBox
  nsDialogs::Show
FunctionEnd

Function un.OptionsLeave
  ${NSD_GetState} $PurgeBox $Purge
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -Command "Get-Process -Name citylive,citylive-settings -ErrorAction SilentlyContinue | Where-Object { $$_.Path -like '$INSTDIR\*' } | Stop-Process -Force"`
  Pop $0
  Sleep 800
  Delete "$INSTDIR\citylive.exe"
  Delete "$INSTDIR\citylive-settings.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\${APP} Settings.lnk"
  RMDir "$SMPROGRAMS\${APP}"
  DeleteRegValue HKCU "${RUNKEY}" "${RUNVALUE}"
  DeleteRegKey HKCU "${UNINSTKEY}"
  ${If} $Purge == ${BST_CHECKED}
    RMDir /r "$LOCALAPPDATA\CityLive"
    RMDir /r "$APPDATA\CityLive"
    RMDir /r "$LOCALAPPDATA\citylive-iso"
  ${EndIf}
SectionEnd

Function un.onInit
  StrCpy $Purge 0
FunctionEnd
