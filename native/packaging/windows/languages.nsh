; ==========================================================================
; Espectral Client — shared strings (English + Spanish).
;
; Included by installer.nsi. Every string the user can see lives here, in
; both languages, so a new message can never ship in one language only.
; ==========================================================================

; ---- English (default / fallback) ----
LangString appRunning            ${LANG_ENGLISH} "Espectral Client is running. It must be closed to replace the launcher.$\n$\nClose it and try again?"
LangString appRunningKillFailed  ${LANG_ENGLISH} "Espectral Client could not be closed. Close it manually and run the installer again."
LangString appClosing            ${LANG_ENGLISH} "Closing the running launcher…"
LangString appClosingForce       ${LANG_ENGLISH} "The launcher did not close on its own; forcing it to stop…"
LangString alreadyInstalled      ${LANG_ENGLISH} "Already installed"
LangString alreadyInstalledBody  ${LANG_ENGLISH} "Espectral Client $R0 is already installed.$\n$\nChoose what to do and click Next."
LangString optInstallKeep        ${LANG_ENGLISH} "Install ${VERSION} (keeps your accounts, instances and settings)"
LangString optUninstallFirst     ${LANG_ENGLISH} "Uninstall the installed version first"
LangString uninstallFailed       ${LANG_ENGLISH} "The previous version could not be uninstalled. Nothing was changed."
LangString createDesktop         ${LANG_ENGLISH} "Create a desktop shortcut"
LangString runApp                ${LANG_ENGLISH} "Run Espectral Client"
LangString removingOldFiles      ${LANG_ENGLISH} "Removing files from the previous (Tauri) launcher…"
LangString instDirTitle          ${LANG_ENGLISH} "Choose where to install Espectral Client"
LangString uninstallConfirm      ${LANG_ENGLISH} "This removes Espectral Client ${VERSION} from this account.$\n$\nYour launcher data (accounts, instances, settings) is kept."
LangString uninstallWait         ${LANG_ENGLISH} "Uninstalling Espectral Client…"
LangString uninstallDone         ${LANG_ENGLISH} "Espectral Client was uninstalled."
LangString keepingUserData       ${LANG_ENGLISH} "Your data folder was left untouched."

; ---- Spanish ----
LangString appRunning            ${LANG_SPANISH} "Espectral Client se está ejecutando. Hay que cerrarlo para reemplazar el lanzador.$\n$\n¿Cerrarlo e intentarlo de nuevo?"
LangString appRunningKillFailed  ${LANG_SPANISH} "No se ha podido cerrar Espectral Client. Ciérrelo manualmente y vuelva a ejecutar el instalador."
LangString appClosing            ${LANG_SPANISH} "Cerrando el lanzador en ejecución…"
LangString appClosingForce       ${LANG_SPANISH} "El lanzador no se cerró por sí solo; forzando su cierre…"
LangString alreadyInstalled      ${LANG_SPANISH} "Ya está instalado"
LangString alreadyInstalledBody  ${LANG_SPANISH} "Espectral Client $R0 ya está instalado.$\n$\nElija qué hacer y pulse Siguiente."
LangString optInstallKeep        ${LANG_SPANISH} "Instalar ${VERSION} (conserva cuentas, instancias y ajustes)"
LangString optUninstallFirst     ${LANG_SPANISH} "Desinstalar primero la versión instalada"
LangString uninstallFailed       ${LANG_SPANISH} "No se ha podido desinstalar la versión anterior. No se ha cambiado nada."
LangString createDesktop         ${LANG_SPANISH} "Crear acceso directo en el escritorio"
LangString runApp                ${LANG_SPANISH} "Ejecutar Espectral Client"
LangString removingOldFiles      ${LANG_SPANISH} "Eliminando archivos del lanzador anterior (Tauri)…"
LangString instDirTitle          ${LANG_SPANISH} "Elija dónde instalar Espectral Client"
LangString uninstallConfirm      ${LANG_SPANISH} "Esto elimina Espectral Client ${VERSION} de esta cuenta.$\n$\nSus datos del lanzador (cuentas, instancias, ajustes) se conservan."
LangString uninstallWait         ${LANG_SPANISH} "Desinstalando Espectral Client…"
LangString uninstallDone         ${LANG_SPANISH} "Espectral Client se ha desinstalado."
LangString keepingUserData       ${LANG_SPANISH} "La carpeta de datos no se ha tocado."
