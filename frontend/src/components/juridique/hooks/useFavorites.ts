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
// Espace "Recherche juridique" — Hook useFavorites
// ============================================================================
// État et logique des documents favoris : persistance localStorage,
// ajout/retrait, panneau d'affichage. Isolé car peu couplé au reste de
// l'état du panneau — seule dépendance externe : le document actuellement
// affiché (`viewedDoc`), passé en paramètre pour calculer `isFavorite`.
// ============================================================================

import { useState, useCallback } from 'react';
import { safeSetJSON, pruneHeavyFieldsForStorage } from '../storage';
import type { ViewedDoc } from '../types';

export function useFavorites(viewedDoc: ViewedDoc | null) {
  const [favorites, setFavorites] = useState<ViewedDoc[]>(() => {
    try { return JSON.parse(localStorage.getItem('juridique_favorites') || '[]'); }
    catch { return []; }
  });
  const [showFavorites, setShowFavorites] = useState(false);

  const toggleFavorite = useCallback((doc: ViewedDoc) => {
    setFavorites((prev) => {
      const exists = prev.some((f) => f.id === doc.id);
      const next = exists
        ? prev.filter((f) => f.id !== doc.id)
        : [{ ...doc, raw: pruneHeavyFieldsForStorage(doc.raw) }, ...prev].slice(0, 50);
      safeSetJSON('juridique_favorites', next);
      return next;
    });
  }, []);

  const isFavorite = viewedDoc
    ? favorites.some((f) => f.id === viewedDoc.id)
    : false;

  return {
    favorites,
    showFavorites, setShowFavorites,
    toggleFavorite,
    isFavorite,
  };
}
