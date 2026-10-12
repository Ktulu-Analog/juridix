// ============================================================================
// Juridix — Assistant IA juridique desktop
// ============================================================================
// Auteur  : Pierre COUGET
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
//           https://www.gnu.org/licenses/agpl-3.0.html
// Année   : 2026
// ----------------------------------------------------------------------------
// Ce fichier fait partie du projet Juridix.
// Vous pouvez le redistribuer et/ou le modifier selon les termes de la
// licence AGPL-3.0 publiée par la Free Software Foundation.
// ============================================================================

// Authentification de l'API HTTP locale (serveur axum).
//
// Le backend génère un jeton aléatoire à chaque lancement et l'expose par IPC
// Tauri (commande `get_api_token`). On le lit une seule fois au démarrage puis
// on enveloppe `window.fetch` pour l'ajouter en en-tête `X-Juridix-Token` à
// toutes les requêtes destinées à l'API. Un site tiers ne peut pas connaître ce
// jeton : il ne peut donc plus piloter l'API locale depuis le navigateur.

import { API_BASE } from './constants';

const TOKEN_HEADER = 'X-Juridix-Token';

let apiToken = '';

function isApiUrl(url: string): boolean {
  // API_BASE vaut '' hors Tauri (dev navigateur + proxy Vite) : les URLs
  // sont alors relatives (« /api-proxy/... », « /api/... »).
  return url.startsWith(`${API_BASE}/api`) || url.startsWith(`${API_BASE}/health`);
}

/**
 * À appeler une fois, AVANT le premier rendu React.
 * Ne rejette jamais : en cas d'échec, l'app démarre mais les appels API
 * recevront un 401 (visible dans la console).
 */
export async function installApiAuth(): Promise<void> {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const tauri = (window as any).__TAURI__;
    if (tauri?.core?.invoke) {
      apiToken = await tauri.core.invoke('get_api_token');
    } else {
      // Développement dans un navigateur, hors Tauri : jeton fixé via
      // JURIDIX_API_TOKEN (build debug du backend) et VITE_JURIDIX_API_TOKEN.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      apiToken = ((import.meta as any).env?.VITE_JURIDIX_API_TOKEN as string | undefined) ?? '';
    }
  } catch (e) {
    console.error('[api-auth] impossible de lire le jeton API :', e);
  }

  if (!apiToken) return;

  const origFetch = window.fetch.bind(window);
  window.fetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url =
      typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (!isApiUrl(url)) return origFetch(input, init);

    const headers = new Headers(
      init?.headers ?? (typeof input === 'object' && 'headers' in input ? input.headers : undefined),
    );
    headers.set(TOKEN_HEADER, apiToken);
    return origFetch(input, { ...init, headers });
  };
}
