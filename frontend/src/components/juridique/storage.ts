// ============================================================================
// Juridix — Espace "Recherche juridique" — Persistance localStorage
// ============================================================================
// Fonctions PURES extraites de JuridiquePanel.tsx (étape 1 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

/**
 * `localStorage.setItem` sûr : une `QuotaExceededError` (ou toute autre
 * erreur de sérialisation) est journalisée plutôt que de remonter et
 * planter tout le rendu React (écran blanc). Utilisé pour toutes les
 * persistances de state applicatif (dossiers, favoris, historique,
 * annotations) — voir `pruneHeavyFieldsForStorage` ci-dessous pour le
 * traitement du cas le plus fréquent (pièces jointes base64 volumineuses).
 */
export function safeSetJSON(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch (e) {
    console.error(`[localStorage] Échec de sauvegarde pour "${key}" :`, e);
  }
}

/**
 * Retire les champs base64 volumineux (`Circulaire.data`, `Debat.data`,
 * `Accord.data` — confirmés par le swagger officiel, "contenu du fichier en
 * base64") d'un objet `raw` avant de le persister en localStorage.
 *
 * Ces champs ne sont JAMAIS utilisés pour le rendu (le PDF se télécharge via
 * une URL dédiée — cf. resolveCircPdfUrl/resolveDebatPdfUrl dans
 * content/html-helpers.ts — pas décodé depuis ce base64), mais ils peuvent
 * peser plusieurs Mo. Comme `saveDossiers`/`toggleFavorite` stockent le
 * `raw` complet de CHAQUE document du dossier/des favoris dans une seule clé
 * localStorage, quelques circulaires/débats avec pièce jointe suffisent à
 * dépasser le quota (5-10 Mo/origine) : `localStorage.setItem` lève alors
 * une `QuotaExceededError` non interceptée qui fait planter tout le rendu
 * React (écran blanc) — précisément le symptôme observé en ajoutant une
 * circulaire à un dossier.
 */
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
