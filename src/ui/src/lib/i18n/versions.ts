/* ==========================================================================
   Espectral Horizon Glass — Version Armory i18n Namespace
   ========================================================================== */

import type { Dict } from '../i18n-dicts';

export const es: Dict = {
  // Page header & titles
  'versions.title': 'Armería de Versiones',
  'versions.tag': 'Armería de Versiones — Espectral Client',
  'versions.subtitle': 'Manifiesto oficial de versiones de Minecraft provisionadas con su runtime JDK correspondiente.',
  'versions.badge': 'MANIFIESTO MOJANG',

  // Filters & search
  'versions.searchPlaceholder': 'Buscar versión (1.21.1, 1.20.4, 25w…)',
  'versions.searchAria': 'Buscar versión por nombre o fecha',
  'versions.filterType': 'Tipo de versión',
  'versions.filterAria': 'Filtrar por tipo de versión',
  'versions.filterAll': 'Todas',
  'versions.filterRelease': 'Releases',
  'versions.filterSnapshot': 'Snapshots',
  'versions.filterBeta': 'Beta',
  'versions.filterAlpha': 'Alpha',
  'versions.filterInstalled': 'Instaladas',

  // Stats & counters
  'versions.showingCount': 'Mostrando {shown} de {total} versiones',
  'versions.totalVersions': '{count} versiones en manifiesto',
  'versions.installedCount': '{count} instaladas',
  'versions.latestRelease': 'Última Release',
  'versions.latestSnapshot': 'Última Snapshot',

  // Version rows
  'versions.typeRelease': 'Release',
  'versions.typeSnapshot': 'Snapshot',
  'versions.typeBeta': 'Old Beta',
  'versions.typeAlpha': 'Old Alpha',
  'versions.installedIn': 'Instalada en {instances}',
  'versions.installedTag': 'En uso',
  'versions.releasedOn': 'Lanzada el {date}',
  'versions.javaRequirement': 'JDK {java}',
  'versions.createInstanceAction': 'Crear instancia',
  'versions.createInstanceTooltip': 'Crear una nueva instancia basada en {version}',
  'versions.openInWizard': 'Iniciar asistente',
  'versions.copiedVersion': 'Versión {version} copiada al portapapeles',

  // Empty & loading states
  'versions.loading': 'Sincronizando manifiesto oficial de Mojang…',
  'versions.error': 'No se pudo cargar el manifiesto de versiones: {error}',
  'versions.retry': 'Reintentar sincronización',
  'versions.noResults': 'Sin resultados para “{query}”',
  'versions.noResultsDesc': 'Prueba ajustando los filtros de búsqueda o el tipo de versión.',
  'versions.loadMore': 'Cargar más versiones ({remaining} restantes)',
  'versions.allLoaded': 'Has llegado al final del manifiesto.',

  // --- Migrated from Legacy ---
  'versions.none': 'Sin versiones disponibles.',
  'versions.requiresJdk': 'Requiere JDK {java} — el launcher provisiona Temurin 8/16/17/21/25',
};

export const en: Dict = {
  // Page header & titles
  'versions.title': 'Version Armory',
  'versions.tag': 'Version Armory — Espectral Client',
  'versions.subtitle': 'Official Minecraft version manifest provisioned with required JDK runtimes.',
  'versions.badge': 'MOJANG MANIFEST',

  // Filters & search
  'versions.searchPlaceholder': 'Search version (1.21.1, 1.20.4, 25w…)',
  'versions.searchAria': 'Search version by name or date',
  'versions.filterType': 'Version type',
  'versions.filterAria': 'Filter by version type',
  'versions.filterAll': 'All',
  'versions.filterRelease': 'Releases',
  'versions.filterSnapshot': 'Snapshots',
  'versions.filterBeta': 'Beta',
  'versions.filterAlpha': 'Alpha',
  'versions.filterInstalled': 'Installed',

  // Stats & counters
  'versions.showingCount': 'Showing {shown} of {total} versions',
  'versions.totalVersions': '{count} versions in manifest',
  'versions.installedCount': '{count} installed',
  'versions.latestRelease': 'Latest Release',
  'versions.latestSnapshot': 'Latest Snapshot',

  // Version rows
  'versions.typeRelease': 'Release',
  'versions.typeSnapshot': 'Snapshot',
  'versions.typeBeta': 'Old Beta',
  'versions.typeAlpha': 'Old Alpha',
  'versions.installedIn': 'Installed in {instances}',
  'versions.installedTag': 'In Use',
  'versions.releasedOn': 'Released on {date}',
  'versions.javaRequirement': 'JDK {java}',
  'versions.createInstanceAction': 'Create instance',
  'versions.createInstanceTooltip': 'Create a new instance based on {version}',
  'versions.openInWizard': 'Launch wizard',
  'versions.copiedVersion': 'Version {version} copied to clipboard',

  // Empty & loading states
  'versions.loading': 'Syncing official Mojang version manifest…',
  'versions.error': 'Could not load the version manifest: {error}',
  'versions.retry': 'Retry synchronization',
  'versions.noResults': 'No results for "{query}"',
  'versions.noResultsDesc': 'Try adjusting your search query or version type filters.',
  'versions.loadMore': 'Load more versions ({remaining} remaining)',
  'versions.allLoaded': 'You have reached the end of the manifest.',

  // --- Migrated from Legacy ---
  'versions.none': 'No versions available.',
  'versions.requiresJdk': 'Requires JDK {java} — the launcher provisions Temurin 8/16/17/21/25',
};
