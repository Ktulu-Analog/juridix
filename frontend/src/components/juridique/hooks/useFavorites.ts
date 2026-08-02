// ============================================================================
// Juridix — Espace "Recherche juridique" — Hook useFavorites
// ============================================================================
// État et logique des documents favoris : persistance localStorage,
// ajout/retrait, panneau d'affichage. Isolé car peu couplé au reste de
// l'état du panneau — seule dépendance externe : le document actuellement
// affiché (`viewedDoc`), passé en paramètre pour calculer `isFavorite`.
//
// Extrait de JuridiquePanel.tsx (étape 4 du refactor — cf. discussion du
// 05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
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
