; ==========================================================================
; Espectral Client — Windows installer (NSIS 3, per-user).
;
; Drop-in successor of the NSIS installer the Tauri 1.6.x launcher ships: it
; installs into the same folder, uses the same uninstall key and the same
; shortcut names, so an old Tauri client's updater can hand its own install
; over to this installer with `setup.exe /P /R /UPDATE /ARGS` and the user
; keeps exactly one Apps & Features entry, one shortcut and all their data.
;
; Only stock NSIS plugins are used (System, nsExec) — no nsis_tauri_utils.
;
; Inputs (makensis -D):
;   VERSION      1.7.0                       (dotted numbers)
;   SRC_EXE      ...\espectral-client.exe    (release build)
;   SRC_BRANDING ...\assets\branding         (dir: *.jar + branding.json)
;   ICON         ...\native\app\icons\icon.ico
;   OUTFILE      ...\EspectralClient_1.7.0_x64-setup.exe
; Optional:
;   HOMEPAGE     product URL (written to the uninstall entry)
; ==========================================================================

Unicode true
ManifestDPIAware true

SetCompressor /SOLID lzma

!include MUI2.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include x64.nsh
!include Win\COM.nsh
!include Win\Propkey.nsh
!include StrFunc.nsh
${StrStr}
${UnStrStr}

; ---- caller-provided ----
!ifndef VERSION
  !error "VERSION is required (-DVERSION=1.7.0)"
!endif
!ifndef SRC_EXE
  !error "SRC_EXE is required (-DSRC_EXE=...\espectral-client.exe)"
!endif
!ifndef SRC_BRANDING
  !error "SRC_BRANDING is required (-DSRC_BRANDING=...\assets\branding)"
!endif
!ifndef ICON
  !error "ICON is required (-DICON=...\icon.ico)"
!endif
!ifndef OUTFILE
  !error "OUTFILE is required (-DOUTFILE=...setup.exe)"
!endif
!ifndef HOMEPAGE
  !define HOMEPAGE "https://espectral.es"
!endif

; ---- product identity (must match the Tauri 1.6.x values) ----
!define PRODUCTNAME      "Espectral Client"
!define MAINBINARYNAME   "espectral-client"
!define BUNDLEID         "es.spectral.launcher"
!define MANUFACTURER     "spectral"            ; 2nd segment of es.spectral.launcher
!define COPYRIGHT        "© 2026 Espectral Client"

!define UNINSTKEY        "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY          "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY   "${MANUKEY}\${PRODUCTNAME}"
; Tauri-only app data (WebView2 profile). The Rust client's own data dir,
; %LOCALAPPDATA%\espectral-client, must never be touched by install/uninstall.
!define TAURI_WEBVIEW_DIR "$LOCALAPPDATA\${BUNDLEID}"
; Placeholder that makes NSIS append the product folder in the directory page.
!define PLACEHOLDER_DIR  "$LOCALAPPDATA\placeholder\${PRODUCTNAME}"

!define MAX_PATH    260
!define SLGP_RAWPATH 0x4

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
InstallDir "${PLACEHOLDER_DIR}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME} installer"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "${MANUFACTURER}"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "${PRODUCTNAME}"
!define MUI_DIRECTORYPAGE_TEXT_TOP "$(instDirTitle)"

; ---- state ----
Var PassiveMode        ; /P  no pages, progress only, auto-close
Var UpdateMode         ; /UPDATE  keep the existing shortcuts
Var NoShortcutMode     ; /NS  no shortcuts at all
Var PrevVersion        ; version found in the uninstall key ("" = fresh install)
Var InstallChoice      ; 1 = overwrite (keep data), 2 = uninstall first
Var CloseAttempts      ; polling counter for CloseRunningLauncher
Var Closed           ; 1 = no launcher left running
Var LauncherRunning  ; 1 = a espectral-client.exe is still alive
Var KeepRadio        ; HWND of the "install and keep data" radio on the maintenance page
Var UninstallRadio   ; HWND of the "uninstall first" radio (kept across pages)

; ==========================================================================
; Pages (GUI mode only; /P and /S skip everything but INSTFILES)
; ==========================================================================
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME

; Maintenance page when a previous install is detected (Tauri had one too).
; `Page custom` is a plain NSIS page, so it consumes neither the MUI_* define
; nor its own unset — set/clear it by hand to keep later pages clean.
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
Page custom PageReinstall PageLeaveReinstall
!undef MUI_PAGE_CUSTOMFUNCTION_PRE

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY

!insertmacro MUI_PAGE_INSTFILES

; Finish page: run the app + optional desktop shortcut, like Tauri's.
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "$(runApp)"
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

; Uninstaller pages
!define MUI_UNCONFIRMPAGE_TEXT_TOP "$(uninstallConfirm)"
!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; English + Spanish; NSIS picks by OS locale, English is the fallback. No
; language dialog — the Tauri installer had displayLanguageSelector off.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Spanish"

; Custom strings MUST come after the MUI_LANGUAGE block: that is what defines
; ${LANG_ENGLISH} / ${LANG_SPANISH}.
!include "languages.nsh"
!include "associations.nsh"

; ==========================================================================
; Helpers
; ==========================================================================
Function SkipIfPassive
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Abort
  ${EndIf}
FunctionEnd
Function un.SkipIfPassive
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Abort
  ${EndIf}
FunctionEnd

; Previous install location recorded by Tauri's (and our) installer.
Function RestorePreviousInstallLocation
  ReadRegStr $4 HKCU "${MANUPRODUCTKEY}" ""
  StrCmp $4 "" +2 0
    StrCpy $INSTDIR $4
FunctionEnd

Function RunMainBinary
  Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
FunctionEnd

; --- maintenance page ------------------------------------------------------
Function PageReinstall
  ReadRegStr $0 HKCU "${UNINSTKEY}" "DisplayVersion"
  ReadRegStr $1 HKCU "${UNINSTKEY}" "UninstallString"
  StrCpy $PrevVersion $0
  StrCmp "$0$1" "" reinstall_done      ; nothing installed -> no page
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Abort                              ; passive/silent always overwrite
  ${EndIf}
  ; The body names the installed version; LangStrings can only read runtime
  ; variables, so hand the version over in $R0.
  StrCpy $R0 $PrevVersion
  !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(alreadyInstalledBody)"
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 36u "$(alreadyInstalledBody)"
  Pop $0
  ${NSD_CreateRadioButton} 10u 44u -10u 12u "$(optInstallKeep)"
  Pop $KeepRadio
  ${NSD_CreateRadioButton} 10u 62u -10u 12u "$(optUninstallFirst)"
  Pop $UninstallRadio
  ${If} $InstallChoice = 2
    ${NSD_SetState} $UninstallRadio ${BST_CHECKED}
  ${Else}
    ${NSD_SetState} $KeepRadio ${BST_CHECKED}
  ${EndIf}
  nsDialogs::Show
reinstall_done:
  ; No previous install: a custom page that shows nothing must skip itself,
  ; otherwise the user gets an empty dialog.
  StrCmp $PrevVersion "" 0 +2
    Abort
FunctionEnd

Function PageLeaveReinstall
  StrCmp $PrevVersion "" reinstall_done
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    StrCpy $InstallChoice 1
    Return
  ${EndIf}
  ; The handles live in their own variables: other pages may have reused
  ; $0..$3 between the show and the leave callback.
  ${NSD_GetState} $UninstallRadio $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $InstallChoice 2
  ${Else}
    StrCpy $InstallChoice 1
  ${EndIf}
  ${If} $InstallChoice = 2
    HideWindow
    ReadRegStr $0 HKCU "${UNINSTKEY}" "UninstallString"
    ReadRegStr $1 HKCU "${MANUPRODUCTKEY}" ""
    StrCmp $1 "" 0 +2
      StrCpy $1 $INSTDIR
    ExecWait '$0 /P _?=$1' $2
    BringToFront
    ${If} $2 <> 0
    ${AndIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      MessageBox MB_ICONEXCLAMATION "$(uninstallFailed)"
      Abort
    ${EndIf}
  ${EndIf}
reinstall_done:
FunctionEnd

; --- shortcut helpers ------------------------------------------------------
; Writes AppUserModelId on a .lnk (same value the Tauri shortcuts carried), so
; the taskbar groups the window under the installed app and a pin survives
; upgrades. Stock Win\COM.nsh + Win\Propkey.nsh only.
!macro SetLnkAppUserModelId shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})'
      ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
      ${If} $2 P<> 0
        System::Call 'Oleaut32::SysAllocString(w "${BUNDLEID}") i.r3'
        System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID})p.r4'
        System::Call '*${SYSSTRUCT_PROPVARIANT}(${VT_BSTR},,&i4 $3)p.r5'
        ${IPropertyStore::SetValue} $2 '($4,$5)'
        System::Call 'Oleaut32::SysFreeString($3)'
        System::Free $4
        System::Free $5
        ${IPropertyStore::Commit} $2 ""
        ${IUnknown::Release} $2 ""
        ${IPersistFile::Save} $1 '("${shortcut}",1)'
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; Unpins a shortcut from Start/taskbar so no dead tile is left behind.
!macro UnpinShortcut shortcut
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_StartMenuPin} ${IID_IStartMenuPinnedList} r0 ""
  ${If} $0 P<> 0
    System::Call 'SHELL32::SHCreateItemFromParsingName(w "${shortcut}", p0, g "${IID_IShellItem}", *p0r1)'
    ${If} $1 P<> 0
      ${IStartMenuPinnedList::RemoveFromList} $0 '(r1)'
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; Sets $9 = 1 when <shortcut> resolves to <target>, else 0.
; Clobbers $0, $1, $2.
!macro IsShortcutTarget shortcut target
  StrCpy $9 0
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READ})'
      System::Alloc ${MAX_PATH}
      Pop $2
      ${IShellLink::GetPath} $0 '(.r2, ${MAX_PATH}, 0, ${SLGP_RAWPATH})'
      ${If} $2 == "${target}"
        StrCpy $9 1
      ${EndIf}
      System::Free $2
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; --- process handling ------------------------------------------------------
; Sets $Closed = 0/1 after asking "is our launcher running?" / closing it.
;
; Graceful first (`taskkill /IM` sends WM_CLOSE, never /F), polled for ~10 s,
; then forced. Only ever targets our own image name: java.exe/javaw.exe run
; Minecraft instances that are designed to outlive the launcher.

; $LauncherRunning = 1 when a process named ${MAINBINARYNAME}.exe exists.
; tasklist output is captured and matched in-script (StrStr) instead of piped
; through `find`: the MSYS/NSIS shell quoting of `cmd /c … | find` is fragile
; and Wine's find.exe does not honor the pipe the same way.
!macro ProbeLauncher prefix
  StrCpy $LauncherRunning 0
  nsExec::ExecToStack 'tasklist /FI "IMAGENAME eq ${MAINBINARYNAME}.exe" /NH'
  Pop $0              ; exit code (0 = ran, whatever it found)
  Pop $1              ; captured stdout
  ${StrStr} $2 "$1" "${MAINBINARYNAME}.exe"
  ${If} $2 != ""
    StrCpy $LauncherRunning 1
  ${EndIf}
!macroend

; Same probe for the uninstaller: StrFunc compiles a separate `un.StrStr`.
!macro un.ProbeLauncher prefix
  StrCpy $LauncherRunning 0
  nsExec::ExecToStack 'tasklist /FI "IMAGENAME eq ${MAINBINARYNAME}.exe" /NH'
  Pop $0
  Pop $1
  ${UnStrStr} $2 "$1" "${MAINBINARYNAME}.exe"
  ${If} $2 != ""
    StrCpy $LauncherRunning 1
  ${EndIf}
!macroend

!macro CloseLauncherBody prefix probe
  StrCpy $Closed 0
  !insertmacro ${probe} ${prefix}
  ${If} $LauncherRunning = 0
    StrCpy $Closed 1
    Return
  ${EndIf}
  IfSilent ${prefix}_kill 0
  ${If} $PassiveMode != 1
    MessageBox MB_OKCANCEL|MB_ICONQUESTION "$(appRunning)" IDOK ${prefix}_kill
    Return                            ; user cancelled -> $Closed stays 0
  ${EndIf}
${prefix}_kill:
  DetailPrint "$(appClosing)"
  StrCpy $CloseAttempts 0
${prefix}_wait:
  nsExec::ExecToStack 'taskkill /IM ${MAINBINARYNAME}.exe'
  Pop $0
  Pop $1
  Sleep 500
  !insertmacro ${probe} ${prefix}
  ${If} $LauncherRunning = 0
    StrCpy $Closed 1
    Return
  ${EndIf}
  IntOp $CloseAttempts $CloseAttempts + 1
  IntCmp $CloseAttempts 20 ${prefix}_force ${prefix}_wait ${prefix}_force
${prefix}_force:
  DetailPrint "$(appClosingForce)"
  nsExec::ExecToStack 'taskkill /F /IM ${MAINBINARYNAME}.exe'
  Pop $0
  Pop $1
  Sleep 1000
  !insertmacro ${probe} ${prefix}
  ${If} $LauncherRunning = 0
    StrCpy $Closed 1
  ${EndIf}
!macroend

Function CloseRunningLauncher
  !insertmacro CloseLauncherBody launcher ProbeLauncher
FunctionEnd

Function un.CloseRunningLauncher
  !insertmacro CloseLauncherBody unlauncher un.ProbeLauncher
FunctionEnd

; ==========================================================================
; .onInit
; ==========================================================================
Function .onInit
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/NS" $R0
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/UPDATE" $R0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  StrCpy $InstallChoice 1
  StrCpy $PrevVersion ""

  SetShellVarContext current
  ; Tauri's installer addressed the 64-bit registry view; match it so the old
  ; uninstall entry, install location and .mrpack handler are found/overwritten.
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}

  ; Keep the path the user picked on the command line (NSIS already parsed
  ; /D=... into $INSTDIR); otherwise restore the previous install location.
  StrCmp $INSTDIR "${PLACEHOLDER_DIR}" 0 oninit_done
  StrCpy $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}"
  Call RestorePreviousInstallLocation
oninit_done:
FunctionEnd

Function un.onInit
  SetShellVarContext current
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  !insertmacro MUI_UNGETLANGUAGE
FunctionEnd

; ==========================================================================
; Install
; ==========================================================================
Section "Install"
  Call CloseRunningLauncher
  ${If} $Closed = 0
    MessageBox MB_ICONSTOP "$(appRunningKillFailed)"
    SetErrorLevel 1
    Quit
  ${EndIf}

  SetOutPath $INSTDIR

  ; ---- main binary (replaces the Tauri exe in place) ----
  File "${SRC_EXE}"

  ; ---- branding (assets/branding: the mod jars + branding.json) ----
  RMDir /r "$INSTDIR\branding"
  CreateDirectory "$INSTDIR\branding"
  SetOutPath "$INSTDIR\branding"
  File /r "${SRC_BRANDING}\*"
  SetOutPath $INSTDIR

  ; ---- drop what the Tauri 1.6.x install put here (no longer used) ----
  DetailPrint "$(removingOldFiles)"
  RMDir /r /REBOOTOK "$INSTDIR\node-bin"
  RMDir /r /REBOOTOK "$INSTDIR\engine"
  RMDir /r /REBOOTOK "$INSTDIR\node_modules"
  Delete /REBOOTOK "$INSTDIR\package.json"
  Delete /REBOOTOK "$INSTDIR\WebView2Loader.dll"
  ; Tauri's WebView2 profile (EBWebView cache). Tauri-only: the Rust client
  ; keeps its data in %LOCALAPPDATA%\espectral-client, never touched here.
  RMDir /r /REBOOTOK "${TAURI_WEBVIEW_DIR}"

  ; ---- uninstaller ----
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; ---- remember where we installed (same key Tauri used) ----
  WriteRegStr HKCU "${MANUPRODUCTKEY}" "" $INSTDIR

  ; ---- Apps & Features entry (same key + value names as the Tauri one) ----
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" '"$INSTDIR\${MAINBINARYNAME}.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" '"$INSTDIR"'
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  WriteRegStr HKCU "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"
  WriteRegStr HKCU "${UNINSTKEY}" "HelpLink" "${HOMEPAGE}"

  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"

  ; ---- .mrpack stays associated with this exe ----
  !insertmacro MRPACK_ASSOCIATE

  ; ---- shortcuts ----
  Call CreateOrUpdateStartMenuShortcut
  ; The finish page offers the desktop shortcut in GUI mode; silent/passive
  ; installs have no page, so create it like Tauri did.
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}

  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function CreateOrUpdateStartMenuShortcut
  ${If} $NoShortcutMode = 1
    Return
  ${EndIf}
  StrCpy $R0 "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${If} $UpdateMode = 1
    ; Upgrading: keep the user's shortcut, only repoint a stale one.
    !insertmacro IsShortcutTarget "$R0" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${If} $9 = 1
      Return
    ${EndIf}
    ${IfNot} ${FileExists} "$R0"
      Return
    ${EndIf}
  ${EndIf}
  Delete "$R0"
  CreateShortcut "$R0" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0 SW_SHOWNORMAL "" "${PRODUCTNAME}"
  !insertmacro SetLnkAppUserModelId "$R0"
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}
  StrCpy $R0 "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$R0"
  CreateShortcut "$R0" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0 SW_SHOWNORMAL "" "${PRODUCTNAME}"
  !insertmacro SetLnkAppUserModelId "$R0"
FunctionEnd

; ==========================================================================
; Relaunch (/R + /ARGS) — the path the 1.6.x Tauri updater takes.
; ==========================================================================
Function .onInstSuccess
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ; Everything after /ARGS, verbatim (quotes preserved), so a path with
      ; spaces survives: /ARGS "C:\a b\x.mrpack" -> "C:\a b\x.mrpack".
      StrCpy $R1 ""
      ${GetOptions} $CMDLINE "/ARGS" $R1
      ${If} ${Errors}
        StrCpy $R1 ""
      ${EndIf}
      ${If} $R1 == ""
        Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
      ${Else}
        Exec '"$INSTDIR\${MAINBINARYNAME}.exe" $R1'
      ${EndIf}
    ${EndIf}
  ${EndIf}
FunctionEnd

; ==========================================================================
; Uninstall
; ==========================================================================
Section "Uninstall"
  DetailPrint "$(uninstallWait)"
  Call un.CloseRunningLauncher
  ${If} $Closed = 0
    MessageBox MB_ICONSTOP "$(appRunningKillFailed)"
    SetErrorLevel 1
    Quit
  ${EndIf}

  ; Shortcuts: only the ones that point at this install.
  !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  ${If} $9 = 1
    !insertmacro UnpinShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${EndIf}
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  ${If} $9 = 1
    !insertmacro UnpinShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}

  ; File association (restores the handler recorded before we took .mrpack).
  !insertmacro MRPACK_UNASSOCIATE

  ; Files.
  RMDir /r "$INSTDIR\branding"
  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  Delete "$INSTDIR\uninstall.exe"
  ; Tauri 1.6.x leftovers, in case this installer never ran on this machine.
  RMDir /r /REBOOTOK "$INSTDIR\node-bin"
  RMDir /r /REBOOTOK "$INSTDIR\engine"
  RMDir /r /REBOOTOK "$INSTDIR\node_modules"
  Delete /REBOOTOK "$INSTDIR\package.json"
  Delete /REBOOTOK "$INSTDIR\WebView2Loader.dll"
  RMDir /r /REBOOTOK "${TAURI_WEBVIEW_DIR}"
  RMDir "$INSTDIR"

  ; Registry: our keys only. %LOCALAPPDATA%\espectral-client is untouched.
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${MANUPRODUCTKEY}"
  DeleteRegKey /ifempty HKCU "${MANUKEY}"

  DetailPrint "$(keepingUserData)"
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd
