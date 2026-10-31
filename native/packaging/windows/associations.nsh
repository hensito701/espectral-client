; ==========================================================================
; .mrpack file association — same registry layout the Tauri NSIS installer
; wrote (FileAssociation.nsh / APP_ASSOCIATE), so an in-place upgrade doesn't
; hand the user a second, different handler and so the uninstaller can undo
; exactly what the old installer (or this one) created.
;
; Per-user (HKCU\Software\Classes), shell context `current`:
;   .mrpack                      (default)  = Espectral Modpack
;   .mrpack                      Espectral Modpack_backup = previous handler
;   Espectral Modpack            (default)  = description
;   Espectral Modpack\DefaultIcon (default) = "$INSTDIR\espectral-client.exe,0"
;   Espectral Modpack\shell     (default)  = open
;   Espectral Modpack\shell\open (default) = "Open with Espectral Client"
;   Espectral Modpack\shell\open\command   = "$INSTDIR\espectral-client.exe " "%1""
; ==========================================================================

; SHChangeNotify flags (values from shlobj_core.h / FileAssociation.nsh).
!define /ifndef SHCNE_ASSOCCHANGED 0x08000000
!define /ifndef SHCNF_FLUSH        0x00001000

!define MIME_ASSOC_EXT       "mrpack"
!define MIME_ASSOC_CLASS     "Espectral Modpack"
!define MIME_ASSOC_DESC      "Modrinth modpack (.mrpack)"
!define MIME_ASSOC_VERB_TEXT "Open with Espectral Client"

!ifndef SHELL_CONTEXT
!define SHELL_CONTEXT HKCU
!endif

!macro MRPACK_ASSOCIATE
  ReadRegStr $R0 SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" ""
  StrCmp $R0 "${MIME_ASSOC_CLASS}" +2
    WriteRegStr SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" "${MIME_ASSOC_CLASS}_backup" "$R0"
  WriteRegStr SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" "" "${MIME_ASSOC_CLASS}"

  WriteRegStr SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}" "" "${MIME_ASSOC_DESC}"
  WriteRegStr SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}\DefaultIcon" "" "$\"$INSTDIR\espectral-client.exe$\",0"
  WriteRegStr SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}\shell" "" "open"
  WriteRegStr SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}\shell\open" "" "${MIME_ASSOC_VERB_TEXT}"
  WriteRegStr SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}\shell\open\command" "" "$\"$INSTDIR\espectral-client.exe$\" $\"%1$\""

  ; Let Explorer pick the change up without a logoff.
  System::Call 'shell32::SHChangeNotify(i ${SHCNE_ASSOCCHANGED}, i ${SHCNF_FLUSH}, p 0, p 0)'
!macroend

!macro MRPACK_UNASSOCIATE
  ReadRegStr $R0 SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" "${MIME_ASSOC_CLASS}_backup"
  StrCmp $R0 "" 0 +2
    DeleteRegValue SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" ""
  StrCmp $R0 "" +2
    WriteRegStr SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" "" "$R0"
  DeleteRegValue SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}" "${MIME_ASSOC_CLASS}_backup"
  ; Leave no empty stub behind (a user's own .mrpack key with values survives).
  DeleteRegKey /ifempty SHELL_CONTEXT "Software\Classes\.${MIME_ASSOC_EXT}"

  DeleteRegKey SHELL_CONTEXT "Software\Classes\${MIME_ASSOC_CLASS}"

  System::Call 'shell32::SHChangeNotify(i ${SHCNE_ASSOCCHANGED}, i ${SHCNF_FLUSH}, p 0, p 0)'
!macroend
