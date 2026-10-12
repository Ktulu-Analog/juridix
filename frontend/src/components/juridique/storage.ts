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

// ============================================================================
// Espace "Recherche juridique" — Persistance localStorage
// ============================================================================



export function safeSetJSON(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch (e) {
    console.error(`[localStorage] Échec de sauvegarde pour "${key}" :`, e);
  }
}

/** Retire les champs base64 volumineux */
export function pruneHeavyFieldsForStorage<T>(raw: T): T {
  if (!raw || typeof raw !== 'object') return raw;
  const strip = (obj: Record<string, unknown>): Record<string, unknown> => {
    if (!('data' in obj)) return obj;
    const { data: _data, ...rest } = obj;
    return rest;
  };
  const top = strip(raw as Record<string, unknown>);
  for (const key of ['circulaire', 'acco', 'debat']) {
    if (top[key] && typeof top[key] === 'object') {
      top[key] = strip(top[key] as Record<string, unknown>);
    }
  }
  return top as T;
}
