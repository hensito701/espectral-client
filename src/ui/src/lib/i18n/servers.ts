/* ==========================================================================
   Espectral Horizon Glass — Server Radar i18n Namespace
   ========================================================================== */

import type { Dict } from '../i18n-dicts';

export const es: Dict = {
  // Page titles & headings
  'servers.title': 'Radar de Servidores',
  'servers.tag': 'Radar de Servidores — Espectral Client',
  'servers.subtitle': 'Telemetría y estado en tiempo real de los nodos de la red Espectral.',
  'servers.badge': 'RED MULTIJUGADOR',

  // Status metrics & summary strip
  'servers.summary': '{online}/{total} nodos activos',
  'servers.totalPlayers': '{players} jugadores conectados',
  'servers.networkHealth': 'Salud de la red',
  'servers.networkOptimal': 'Óptima',
  'servers.networkDegraded': 'Degradada',
  'servers.networkOffline': 'Sin conexión',
  'servers.lastUpdated': 'Actualizado {time}',
  'servers.refresh': 'Escanear radar',
  'servers.refreshing': 'Escaneando…',
  'servers.autoPollNote': 'Sondeo automático cada 60s',

  // Card & beacon telemetry
  'servers.onlineNodes': 'Nodos en línea',
  'servers.offlineNodes': 'Nodos fuera de línea',
  'servers.allOnline': 'Todos los servidores están operativos',
  'servers.someOffline': 'Algunos servidores no responden',
  'servers.allOffline': 'No se detecta conexión con los servidores',
  'servers.empty': 'No hay servidores configurados en el radar.',
  'servers.errorTitle': 'Fallo de sondeo de telemetría',
  'servers.retry': 'Reintentar sondeo',

  // Custom servers & instance hints
  'servers.customTitle': 'Servidores personalizados',
  'servers.customDesc': 'Para conectarte a servidores externos, añade las IPs directamente en la configuración de servidores de tu instancia favorita.',
  'servers.manageInstances': 'Gestionar instancias',
  'servers.openSettings': 'Ir a Ajustes',

  // Quick actions
  'servers.copyIp': 'Copiar IP directa',
  'servers.copyHost': 'Copiar dominio',
  'servers.copied': '¡Copiado!',
  'servers.directConnectHint': 'Pega esta dirección en la lista de servidores de Minecraft para entrar de inmediato.',

  // --- Migrated from Legacy ---
  'server.addressCopied': 'Dirección copiada',
  'server.copied': '✓ Copiado',
  'server.copy': 'Copiar',
  'server.copyIp': 'Copiar IP',
  'server.ipCopied': 'IP copiada',
  'server.offline': 'Sin conexión',
  'server.online': 'Servidor en línea',
  'server.players': 'Jugadores',
  'servers.noData': 'Sin datos',
  'servers.noServers': 'Sin datos de servidores.',
};

export const en: Dict = {
  // Page titles & headings
  'servers.title': 'Server Radar',
  'servers.tag': 'Server Radar — Espectral Client',
  'servers.subtitle': 'Real-time telemetry and health monitoring for Espectral network nodes.',
  'servers.badge': 'MULTIPLAYER MESH',

  // Status metrics & summary strip
  'servers.summary': '{online}/{total} active nodes',
  'servers.totalPlayers': '{players} players connected',
  'servers.networkHealth': 'Network Health',
  'servers.networkOptimal': 'Optimal',
  'servers.networkDegraded': 'Degraded',
  'servers.networkOffline': 'Offline',
  'servers.lastUpdated': 'Updated {time}',
  'servers.refresh': 'Scan radar',
  'servers.refreshing': 'Scanning…',
  'servers.autoPollNote': 'Auto-polled every 60s',

  // Card & beacon telemetry
  'servers.onlineNodes': 'Online nodes',
  'servers.offlineNodes': 'Offline nodes',
  'servers.allOnline': 'All servers are operational',
  'servers.someOffline': 'Some servers are unreachable',
  'servers.allOffline': 'Cannot connect to server nodes',
  'servers.empty': 'No servers configured in radar.',
  'servers.errorTitle': 'Telemetry polling error',
  'servers.retry': 'Retry poll',

  // Custom servers & instance hints
  'servers.customTitle': 'Custom Servers',
  'servers.customDesc': 'To connect to third-party servers, add their IP addresses directly in your chosen instance servers configuration.',
  'servers.manageInstances': 'Manage instances',
  'servers.openSettings': 'Go to Settings',

  // Quick actions
  'servers.copyIp': 'Copy direct IP',
  'servers.copyHost': 'Copy domain',
  'servers.copied': 'Copied!',
  'servers.directConnectHint': 'Paste this address into Minecraft server list to join immediately.',

  // --- Migrated from Legacy ---
  'server.addressCopied': 'Address copied',
  'server.copied': '✓ Copied',
  'server.copy': 'Copy',
  'server.copyIp': 'Copy IP',
  'server.ipCopied': 'IP copied',
  'server.offline': 'Offline',
  'server.online': 'Server online',
  'server.players': 'Players',
  'servers.noData': 'No data',
  'servers.noServers': 'No server data.',
};
