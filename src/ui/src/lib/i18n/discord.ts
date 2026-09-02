/* ==========================================================================
   Espectral Horizon Glass — Discord OAuth & Login Gate i18n Strings
   Spanish and English translations for identity authentication and gate.
   ========================================================================== */

import type { Dict } from '../i18n-dicts';

export const es: Dict = {
  'gate.title': 'Identidad Espectral',
  'gate.welcome': 'Bienvenido a Espectral',
  'gate.subtitle': 'Inicia sesión con Discord para sincronizar tus rangos de donador, cosméticos y perfil en la red.',
  'gate.discord': 'Continuar con Discord',
  'gate.anon': 'Continuar sin cuenta',
  'gate.footnote': 'Tu cuenta de Discord solo se utiliza para el registro de usuarios y donantes. Nunca solicitamos acceso a mensajes ni datos privados.',
  'gate.connecting': 'Conectando con Discord...',
  'gate.waiting': 'Esperando autorización en el navegador...',
  'gate.success': '¡Bienvenido, {name}!',
  'gate.error': 'Error al iniciar sesión con Discord. Inténtalo de nuevo.',
  'gate.retry': 'Reintentar',
  'gate.authedAs': 'Sesión iniciada como',
  'gate.logout': 'Cerrar sesión de Discord',
  'gate.status.loading': 'Verificando sesión...',
  'gate.badge': 'DISCORD OAUTH',
  'gate.supporterRegistry': 'Registro de Donantes',
};

export const en: Dict = {
  'gate.title': 'Espectral Identity',
  'gate.welcome': 'Welcome to Espectral',
  'gate.subtitle': 'Sign in with Discord to sync your donor ranks, cosmetics, and network profile.',
  'gate.discord': 'Continue with Discord',
  'gate.anon': 'Continue without account',
  'gate.footnote': 'Your Discord account is only used for the supporter and user registry. We never request access to private messages or data.',
  'gate.connecting': 'Connecting to Discord...',
  'gate.waiting': 'Waiting for authorization in browser...',
  'gate.success': 'Welcome, {name}!',
  'gate.error': 'Failed to sign in with Discord. Please try again.',
  'gate.retry': 'Retry',
  'gate.authedAs': 'Signed in as',
  'gate.logout': 'Sign out of Discord',
  'gate.status.loading': 'Verifying session...',
  'gate.badge': 'DISCORD OAUTH',
  'gate.supporterRegistry': 'Supporter Registry',
};
