/* ==========================================================================
   Espectral Horizon Glass — Common / Shared i18n Strings
   Actions, status indicators, navigation labels, and global telemetry.
   ========================================================================== */

import type { Dict } from '../i18n-dicts';

export const es: Dict = {
  // --- Actions ---
  'actions.play': 'Jugar',
  'actions.back': 'Volver',
  'actions.cancel': 'Cancelar',
  'actions.save': 'Guardar',
  'actions.delete': 'Eliminar',
  'actions.confirm': 'Confirmar',
  'actions.close': 'Cerrar',
  'actions.search': 'Buscar',
  'actions.loading': 'Cargando…',
  'actions.retry': 'Reintentar',
  'actions.open': 'Abrir',
  'actions.copy': 'Copiar',
  'actions.copied': '¡Copiado!',
  'actions.yes': 'Sí',
  'actions.no': 'No',
  'actions.on': 'Activado',
  'actions.off': 'Desactivado',
  'actions.refresh': 'Actualizar',
  'actions.edit': 'Editar',
  'actions.create': 'Crear',
  'actions.install': 'Instalar',
  'actions.uninstall': 'Desinstalar',
  'actions.explore': 'Explorar',
  'actions.launch': 'Lanzar',
  'actions.stop': 'Detener',

  // --- Status ---
  'status.online': 'En línea',
  'status.offline': 'Sin conexión',
  'status.running': 'En ejecución',
  'status.stopped': 'Detenido',
  'status.installing': 'Instalando',
  'status.error': 'Error',
  'status.unknown': 'Desconocido',
  'status.ready': 'Listo',
  'status.pending': 'Pendiente',

  // --- Updater (top-bar CTA; see components/TopChrome.svelte) ---
  'update.badge': 'Actualizar a {version}',
  'update.badgeAria': 'Actualizar Espectral Client a la versión {version}',
  'update.working': 'Descargando actualización…',
  'update.progress': '{pct} % — la app se reiniciará al terminar',
  'update.installing': 'Instalando…',

  // --- Navigation Labels ---
  'nav.home': 'Inicio',
  'nav.servers': 'Servidores',
  'nav.library': 'Biblioteca',
  'nav.versions': 'Versiones',
  'nav.import': 'Importar',
  'nav.mods': 'Mods',
  'nav.client': 'Cliente',
  'nav.account': 'Cuenta',
  'nav.settings': 'Ajustes',
  'nav.donations': 'Donaciones',

  // --- Time Shortcuts ---
  'time.now': 'ahora',
  'time.secondsAgo': 'hace {n} s',
  'time.minutesAgo': 'hace {n} min',
  'time.hoursAgo': 'hace {n} h',
  'time.daysAgo': 'hace {n} d',

  // --- Horizon Hero & Rail ---
  'horizon.explore': 'Explorar catálogo',
  'horizon.newInstance': 'Nueva Instancia',
  'horizon.lastPlayed': 'Última partida: {time}',
  'horizon.playtime': 'Tiempo jugado: {time}',
  'horizon.scrollHint': 'Usa ← → para navegar entre instancias',
  'horizon.compactView': 'Vista compacta',
  'horizon.heroTagline': 'Tu centro de comando para Minecraft competitivo',

  // --- Telemetry & Boot Phases ---
  'telemetry.ready': 'Listo para jugar',
  'telemetry.launching': 'Iniciando Minecraft…',
  'telemetry.phase.classpath': 'Resolviendo classpath',
  'telemetry.phase.jvm': 'Iniciando JVM',
  'telemetry.phase.aot': 'Aplicando optimización AOT',
  'telemetry.phase.menu': 'Cargando menú principal',
  'telemetry.ramGauge': 'RAM: {used} / {total} MB',
  'telemetry.aotReady': 'AOT optimizado ✓',
  'telemetry.aotTraining': 'Entrenando AOT…',
  'telemetry.dryRun': 'Simular arranque',

  // --- Density Toggle ---
  'density.compact': 'Modo compacto',
  'density.spacious': 'Modo espacioso',
  'density.toggle': 'Alternar densidad de vista',

  // --- Global Hotkeys ---
  'hotkey.command': 'Comandos',
  'hotkey.search': 'Buscar',
  'hotkey.logs': 'Logs',
  'hotkey.back': 'Volver',
  'hotkey.play': 'Jugar',
  'hotkey.density': 'Densidad',
  'hotkey.close': 'Cerrar',

  // --- Migrated from Legacy ---
  'api.network': 'Error de red al contactar con el motor',
  'api.timeout': 'Tiempo de espera agotado ({ms} ms)',
  'common.close': 'Cerrar',
  'common.loading': 'Cargando…',
  'common.online': 'en línea',
  'common.saving': 'Guardando…',
  'lang.toEn': 'Cambiar a inglés',
  'lang.toEs': 'Cambiar a español',
  'nav.instances': 'Instancias',
  'nav.noAccount': 'Sin cuenta',
  'themeToggle.toDark': 'Activar modo oscuro',
  'themeToggle.toLight': 'Activar modo claro',
  'time.ago': 'hace {n} {unit}',
  'time.in': 'en {n} {unit}',
  'time.unit.month.one': 'mes',
  'time.unit.month.other': 'meses',
  'time.unit.year.one': 'año',
  'time.unit.year.other': 'años',
  'toast.mrpackAlready': 'El modpack "{name}" ya existe',
  'toast.mrpackError': 'No se pudo instalar el modpack: {error}',
  'toast.mrpackInstalled': 'Modpack "{name}" instalado',
};

export const en: Dict = {
  // --- Actions ---
  'actions.play': 'Play',
  'actions.back': 'Back',
  'actions.cancel': 'Cancel',
  'actions.save': 'Save',
  'actions.delete': 'Delete',
  'actions.confirm': 'Confirm',
  'actions.close': 'Close',
  'actions.search': 'Search',
  'actions.loading': 'Loading…',
  'actions.retry': 'Retry',
  'actions.open': 'Open',
  'actions.copy': 'Copy',
  'actions.copied': 'Copied!',
  'actions.yes': 'Yes',
  'actions.no': 'No',
  'actions.on': 'On',
  'actions.off': 'Off',
  'actions.refresh': 'Refresh',
  'actions.edit': 'Edit',
  'actions.create': 'Create',
  'actions.install': 'Install',
  'actions.uninstall': 'Uninstall',
  'actions.explore': 'Explore',
  'actions.launch': 'Launch',
  'actions.stop': 'Stop',

  // --- Status ---
  'status.online': 'Online',
  'status.offline': 'Offline',
  'status.running': 'Running',
  'status.stopped': 'Stopped',
  'status.installing': 'Installing',
  'status.error': 'Error',
  'status.unknown': 'Unknown',
  'status.ready': 'Ready',
  'status.pending': 'Pending',

  // --- Updater (top-bar CTA; see components/TopChrome.svelte) ---
  'update.badge': 'Update to {version}',
  'update.badgeAria': 'Update Espectral Client to version {version}',
  'update.working': 'Downloading update…',
  'update.progress': '{pct} % — the app will restart when done',
  'update.installing': 'Installing…',

  // --- Navigation Labels ---
  'nav.home': 'Home',
  'nav.servers': 'Servers',
  'nav.library': 'Library',
  'nav.versions': 'Versions',
  'nav.import': 'Import',
  'nav.mods': 'Mods',
  'nav.client': 'Client',
  'nav.account': 'Account',
  'nav.settings': 'Settings',
  'nav.donations': 'Donations',

  // --- Time Shortcuts ---
  'time.now': 'now',
  'time.secondsAgo': '{n}s ago',
  'time.minutesAgo': '{n}m ago',
  'time.hoursAgo': '{n}h ago',
  'time.daysAgo': '{n}d ago',

  // --- Horizon Hero & Rail ---
  'horizon.explore': 'Explore Catalog',
  'horizon.newInstance': 'New Instance',
  'horizon.lastPlayed': 'Last played: {time}',
  'horizon.playtime': 'Playtime: {time}',
  'horizon.scrollHint': 'Use ← → to navigate between instances',
  'horizon.compactView': 'Compact view',
  'horizon.heroTagline': 'Your command deck for competitive Minecraft',

  // --- Telemetry & Boot Phases ---
  'telemetry.ready': 'Ready to play',
  'telemetry.launching': 'Starting Minecraft…',
  'telemetry.phase.classpath': 'Resolving classpath',
  'telemetry.phase.jvm': 'Starting JVM',
  'telemetry.phase.aot': 'Applying AOT optimization',
  'telemetry.phase.menu': 'Loading main menu',
  'telemetry.ramGauge': 'RAM: {used} / {total} MB',
  'telemetry.aotReady': 'AOT optimized ✓',
  'telemetry.aotTraining': 'Training AOT…',
  'telemetry.dryRun': 'Simulate launch',

  // --- Density Toggle ---
  'density.compact': 'Compact mode',
  'density.spacious': 'Spacious mode',
  'density.toggle': 'Toggle view density',

  // --- Global Hotkeys ---
  'hotkey.command': 'Commands',
  'hotkey.search': 'Search',
  'hotkey.logs': 'Logs',
  'hotkey.back': 'Back',
  'hotkey.play': 'Play',
  'hotkey.density': 'Density',
  'hotkey.close': 'Close',

  // --- Migrated from Legacy ---
  'api.network': 'Network error contacting engine',
  'api.timeout': 'Request timed out ({ms} ms)',
  'common.close': 'Close',
  'common.loading': 'Loading…',
  'common.online': 'online',
  'common.saving': 'Saving…',
  'lang.toEn': 'Switch to English',
  'lang.toEs': 'Switch to Spanish',
  'nav.instances': 'Instances',
  'nav.noAccount': 'No account',
  'themeToggle.toDark': 'Switch to dark mode',
  'themeToggle.toLight': 'Switch to light mode',
  'time.ago': '{n} {unit} ago',
  'time.in': 'in {n} {unit}',
  'time.unit.month.one': 'month',
  'time.unit.month.other': 'months',
  'time.unit.year.one': 'year',
  'time.unit.year.other': 'years',
  'toast.mrpackAlready': 'Modpack "{name}" already exists',
  'toast.mrpackError': 'Could not install the modpack: {error}',
  'toast.mrpackInstalled': 'Modpack "{name}" installed',
};
