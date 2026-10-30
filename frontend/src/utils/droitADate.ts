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
// Juridix — « Droit applicable au » : date des faits partagée par le chat, la
// vérification des citations et la recherche dans les codes.
//
// Volontairement NON persistée : une date des faits oubliée d'une session à
// l'autre fausserait silencieusement les réponses suivantes. Elle est toujours
// affichée là où elle s'applique (puce dans la zone de saisie) et se vide d'un clic.
// ============================================================================

import { useSyncExternalStore } from 'react';

let current: string | null = null;
const listeners = new Set<() => void>();

/** Années acceptées : au-delà, c'est une saisie intermédiaire ou une faute de frappe (« 0002-05-12 »). */
export const ANNEE_MIN = 1800;
export const ANNEE_MAX = 2200;

/** « AAAA-MM-JJ » correspondant à un vrai jour du calendrier (pas de 31 février), année plausible. */
export function isValidIsoDate(s: string | null | undefined): s is string {
  if (!s || !/^\d{4}-\d{2}-\d{2}$/.test(s)) return false;
  const [y, m, d] = s.split('-').map(Number);
  if (y < ANNEE_MIN || y > ANNEE_MAX) return false;
  const dt = new Date(Date.UTC(y, m - 1, d));
  return dt.getUTCFullYear() === y && dt.getUTCMonth() === m - 1 && dt.getUTCDate() === d;
}

export function getDroitADate(): string | null {
  return current;
}

/** Date invalide ou vide : revient au droit actuel. */
export function setDroitADate(date: string | null): void {
  const next = isValidIsoDate(date) ? date : null;
  if (next === current) return;
  current = next;
  listeners.forEach(l => l());
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => { listeners.delete(cb); };
}

export function useDroitADate(): string | null {
  return useSyncExternalStore(subscribe, getDroitADate, getDroitADate);
}

/** « 2018-05-12 » → « 12/05/2018 ». */
export function formatDroitADate(iso: string): string {
  const [y, m, d] = iso.split('-');
  return y && m && d ? `${d}/${m}/${y}` : iso;
}
