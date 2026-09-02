/* ==========================================================================
   Espectral Horizon Glass — Import Center i18n Strings
   Namespace: import.* (Modpack .mrpack installation & launcher profile migration)
   ========================================================================== */

import type { Dict } from '../i18n-dicts';

export const es: Dict = {
  // Headings & Meta
  'import.pageTitle': 'Centro de Importación',
  'import.pageSubtitle': 'Migración de datos, modpacks .mrpack y perfiles de otros lanzadores',
  'import.tag': 'Importación — Espectral Client',

  // Lane Headers
  'import.laneA.badge': 'Modpack',
  'import.laneA.title': 'Instalar Modpack .mrpack',
  'import.laneA.description':
    'Instalación automatizada desde archivos .mrpack de Modrinth con descarga concurrente y verificación SHA-1.',

  'import.laneB.badge': 'Perfiles',
  'import.laneB.title': 'Perfil de otro lanzador',
  'import.laneB.description':
    'Transfiere servidores, configuración de teclas y opciones desde Vanilla, FastClient o Lunar Client.',

  // Lane A: .mrpack
  'import.mrpack.dropTitle': 'Arrastra tu archivo .mrpack aquí',
  'import.mrpack.dropSubtitle': 'o haz clic en esta tarjeta para explorar en tu equipo',
  'import.mrpack.browseButton': 'Examinar archivo .mrpack',
  'import.mrpack.selectedFile': 'Archivo seleccionado:',
  'import.mrpack.clearFile': 'Quitar archivo',
  'import.mrpack.memoryLabel': 'Memoria RAM asignada (MB)',
  'import.mrpack.memoryHint':
    'Cantidad de memoria inicial para la nueva instancia creada (por defecto 3072 MB / 3 GB).',
  'import.mrpack.installButton': 'Instalar Modpack',
  'import.mrpack.installing': 'Instalando modpack…',
  'import.mrpack.selectFirst': 'Selecciona o arrastra un archivo .mrpack primero.',

  // Progress states
  'import.mrpack.phaseIdle': 'Listo para comenzar la instalación.',
  'import.mrpack.phasePost': 'Analizando paquete y preparando la instancia…',
  'import.mrpack.phaseFiles': 'Descargando mods ({done} de {total})…',
  'import.mrpack.phaseOverrides': 'Extrayendo configuraciones y recursos ({done} de {total})…',
  'import.mrpack.phaseDone': '¡Modpack instalado correctamente!',
  'import.mrpack.phaseError': 'Error al instalar el modpack: {error}',
  'import.mrpack.alreadyExists': 'La instancia "{name}" ya existía en tu catálogo.',

  // Success Card Lane A
  'import.mrpack.successTitle': '¡Instalación completada!',
  'import.mrpack.successDesc':
    'La instancia "{name}" se ha creado con éxito y está lista para jugar.',
  'import.mrpack.viewInstance': 'Abrir instancia',
  'import.mrpack.importAnother': 'Instalar otro modpack',

  // Lane B: Launcher Profiles
  'import.profile.sourceListTitle': 'Lanzadores y perfiles detectados',
  'import.profile.sourceListSubtitle':
    'Selecciona el origen desde el que deseas transferir opciones y servidores:',
  'import.profile.noSourcesFound':
    'No se han detectado perfiles ni lanzadores instalados en tu sistema (%APPDATA%\\.minecraft, FastClient o Lunar).',
  'import.profile.loadingSources': 'Escaneando lanzadores instalados en el sistema…',
  'import.profile.errorSources': 'No se pudieron detectar orígenes: {error}',
  'import.profile.retrySources': 'Reintentar escaneo',

  'import.profile.targetInstance': 'Instancia de destino',
  'import.profile.targetHint':
    'La instancia de Espectral donde se aplicarán los servidores y opciones importados.',
  'import.profile.selectTarget': '— Selecciona una instancia —',
  'import.profile.noInstances':
    'No tienes instancias creadas. Crea una instancia primero para poder transferirle ajustes.',
  'import.profile.createInstance': 'Crear nueva instancia',

  'import.profile.optionsCount': '{count} opciones',
  'import.profile.serversCount': '{count} servidores',
  'import.profile.lunarRam': 'RAM configurada: {ram} MB',
  'import.profile.lunarFov': 'FOV: {fov}°',
  'import.profile.lunarMods': 'Mods activos en Lunar: {count}',
  'import.profile.lunarTokenShield':
    'Token Shield Activo: Nunca altera ni invalida la cadena de tokens de Lunar Client.',

  'import.profile.overwriteTitle': 'Política de sobrescritura',
  'import.profile.policyNever': 'Nunca sobrescribir archivos existentes',
  'import.profile.policyNeverDesc':
    'Solo copia archivos u opciones que no existan en la instancia. Protege tu configuración actual.',
  'import.profile.policyIfOlder': 'Sobrescribir si el origen es más reciente',
  'import.profile.policyIfOlderDesc':
    'Actualiza los archivos existentes con los del origen. Genera automáticamente una copia de respaldo servers.dat.bak.',

  'import.profile.importButton': 'Importar configuración',
  'import.profile.importing': 'Transfiriendo datos…',
  'import.profile.selectSourceTarget':
    'Selecciona un lanzador origen y una instancia destino para continuar.',

  // Result Card Lane B
  'import.result.title': 'Resultado de la transferencia',
  'import.result.copied': 'Archivos copiados ({count})',
  'import.result.nothingCopied': 'No fue necesario copiar ningún archivo.',
  'import.result.skipped': 'Archivos omitidos ({count})',
  'import.result.nothingSkipped': 'Ningún archivo omitido.',
  'import.result.servers': 'Servidores detectados ({count})',
  'import.result.noServers': 'No se encontraron servidores en el perfil origen.',
  'import.result.hasIcon': 'Con icono',
  'import.result.optionsKeys': 'Opciones importadas ({count} claves)',
  'import.result.newImport': 'Importar desde otro perfil',

  // Recent imports history
  'import.recent.title': 'Historial de importaciones',
  'import.recent.badge': 'Esta sesión',
  'import.recent.empty': 'Aún no se han realizado importaciones en esta sesión.',
  'import.recent.typeMrpack': 'Modpack .mrpack',
  'import.recent.typeProfile': 'Perfil de {kind}',
  'import.recent.copiedSummary': '{copied} archivos copiados, {servers} servidores',
  'import.recent.openInstance': 'Ver instancia',
};

export const en: Dict = {
  // Headings & Meta
  'import.pageTitle': 'Import Center',
  'import.pageSubtitle': 'Data migration, .mrpack modpacks, and profiles from other launchers',
  'import.tag': 'Import — Espectral Client',

  // Lane Headers
  'import.laneA.badge': 'Modpack',
  'import.laneA.title': 'Install .mrpack Modpack',
  'import.laneA.description':
    'Automated installation from Modrinth .mrpack packages with concurrent mod downloads and SHA-1 verification.',

  'import.laneB.badge': 'Profiles',
  'import.laneB.title': 'Launcher Profile',
  'import.laneB.description':
    'Transfer servers, keybinds, and options from Vanilla, FastClient, or Lunar Client.',

  // Lane A: .mrpack
  'import.mrpack.dropTitle': 'Drop your .mrpack file here',
  'import.mrpack.dropSubtitle': 'or click on this card to browse your files',
  'import.mrpack.browseButton': 'Browse .mrpack file',
  'import.mrpack.selectedFile': 'Selected file:',
  'import.mrpack.clearFile': 'Remove file',
  'import.mrpack.memoryLabel': 'Allocated RAM (MB)',
  'import.mrpack.memoryHint':
    'Initial RAM allocated to the newly created instance (default: 3072 MB / 3 GB).',
  'import.mrpack.installButton': 'Install Modpack',
  'import.mrpack.installing': 'Installing modpack…',
  'import.mrpack.selectFirst': 'Please select or drop a .mrpack file first.',

  // Progress states
  'import.mrpack.phaseIdle': 'Ready to begin installation.',
  'import.mrpack.phasePost': 'Analyzing package and preparing instance…',
  'import.mrpack.phaseFiles': 'Downloading mods ({done} of {total})…',
  'import.mrpack.phaseOverrides': 'Extracting configurations and resources ({done} of {total})…',
  'import.mrpack.phaseDone': 'Modpack installed successfully!',
  'import.mrpack.phaseError': 'Failed to install modpack: {error}',
  'import.mrpack.alreadyExists': 'Instance "{name}" already existed in your library.',

  // Success Card Lane A
  'import.mrpack.successTitle': 'Installation Complete!',
  'import.mrpack.successDesc':
    'Instance "{name}" has been created successfully and is ready to launch.',
  'import.mrpack.viewInstance': 'Open instance',
  'import.mrpack.importAnother': 'Install another modpack',

  // Lane B: Launcher Profiles
  'import.profile.sourceListTitle': 'Detected Launchers & Profiles',
  'import.profile.sourceListSubtitle':
    'Select the source launcher to transfer options and servers from:',
  'import.profile.noSourcesFound':
    'No profiles or installed launchers were detected on your system (%APPDATA%\\.minecraft, FastClient, or Lunar).',
  'import.profile.loadingSources': 'Scanning for installed launchers on system…',
  'import.profile.errorSources': 'Could not detect sources: {error}',
  'import.profile.retrySources': 'Retry scan',

  'import.profile.targetInstance': 'Target Instance',
  'import.profile.targetHint':
    'The Espectral instance where imported servers and options will be applied.',
  'import.profile.selectTarget': '— Select an instance —',
  'import.profile.noInstances':
    'No instances found. Create an instance first to transfer settings into it.',
  'import.profile.createInstance': 'Create new instance',

  'import.profile.optionsCount': '{count} options',
  'import.profile.serversCount': '{count} servers',
  'import.profile.lunarRam': 'Configured RAM: {ram} MB',
  'import.profile.lunarFov': 'FOV: {fov}°',
  'import.profile.lunarMods': 'Active Lunar mods: {count}',
  'import.profile.lunarTokenShield':
    'Token Shield Active: Preserves Lunar Client authentication tokens and chain completely untouched.',

  'import.profile.overwriteTitle': 'Overwrite Policy',
  'import.profile.policyNever': 'Never overwrite existing files',
  'import.profile.policyNeverDesc':
    'Only copies files or options missing from the instance. Protects your existing setup.',
  'import.profile.policyIfOlder': 'Overwrite if source is newer',
  'import.profile.policyIfOlderDesc':
    'Updates existing files with newer source versions. Automatically creates a servers.dat.bak backup file.',

  'import.profile.importButton': 'Import Settings',
  'import.profile.importing': 'Transferring data…',
  'import.profile.selectSourceTarget':
    'Select a source launcher and a target instance to proceed.',

  // Result Card Lane B
  'import.result.title': 'Transfer Result',
  'import.result.copied': 'Copied files ({count})',
  'import.result.nothingCopied': 'No files needed copying.',
  'import.result.skipped': 'Skipped files ({count})',
  'import.result.nothingSkipped': 'No files were skipped.',
  'import.result.servers': 'Detected servers ({count})',
  'import.result.noServers': 'No servers found in the source profile.',
  'import.result.hasIcon': 'With icon',
  'import.result.optionsKeys': 'Imported options ({count} keys)',
  'import.result.newImport': 'Import from another profile',

  // Recent imports history
  'import.recent.title': 'Import History',
  'import.recent.badge': 'This session',
  'import.recent.empty': 'No imports have been performed in this session yet.',
  'import.recent.typeMrpack': '.mrpack Modpack',
  'import.recent.typeProfile': '{kind} Profile',
  'import.recent.copiedSummary': '{copied} files copied, {servers} servers',
  'import.recent.openInstance': 'View instance',
};
