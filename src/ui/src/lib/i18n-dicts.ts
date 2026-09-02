/* ==========================================================================
   Espectral Horizon Glass — Root i18n Dictionary Hub
   Aggregates modular feature namespaces.
   ========================================================================== */

import { es as commonEs, en as commonEn } from './i18n/common';
import { es as homeEs, en as homeEn } from './i18n/home';
import { es as instanceEs, en as instanceEn } from './i18n/instance';
import { es as serversEs, en as serversEn } from './i18n/servers';
import { es as versionsEs, en as versionsEn } from './i18n/versions';
import { es as importEs, en as importEn } from './i18n/import';
import { es as modsEs, en as modsEn } from './i18n/mods';
import { es as clientEs, en as clientEn } from './i18n/client';
import { es as accountEs, en as accountEn } from './i18n/account';
import { es as settingsEs, en as settingsEn } from './i18n/settings';
import { es as donationsEs, en as donationsEn } from './i18n/donations';
import { es as paletteEs, en as paletteEn } from './i18n/palette';
import { es as piplogEs, en as piplogEn } from './i18n/piplog';
import { es as discordEs, en as discordEn } from './i18n/discord';

export type Dict = Record<string, string>;

/**
 * Spanish dictionary aggregated from modular namespaces.
 */
export const es: Dict = {
  ...commonEs,
  ...homeEs,
  ...instanceEs,
  ...serversEs,
  ...versionsEs,
  ...importEs,
  ...modsEs,
  ...clientEs,
  ...accountEs,
  ...settingsEs,
  ...donationsEs,
  ...paletteEs,
  ...piplogEs,
  ...discordEs,
  'home.capsuleTiming': 'Preparación {prep}s · JVM {jvm}s',
};

/**
 * English dictionary aggregated from modular namespaces.
 */
export const en: Dict = {
  ...commonEn,
  ...homeEn,
  ...instanceEn,
  ...serversEn,
  ...versionsEn,
  ...importEn,
  ...modsEn,
  ...clientEn,
  ...accountEn,
  ...settingsEn,
  ...donationsEn,
  ...paletteEn,
  ...piplogEn,
  ...discordEn,
  'home.capsuleTiming': 'Prep {prep}s · JVM {jvm}s',
};
