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
// Juridix — Dates de versions d'un article (fonctions pures)
// ============================================================================
// Sert au bandeau « Droit applicable au… » de la fiche article : la version
// affichée est-elle celle qui était en vigueur à la date des faits ?
// ============================================================================

/** Année à partir de laquelle une date de fin signifie « sans fin » (Légifrance : 2999-01-01). */
const ANNEE_SANS_FIN = 2900;

/**
 * Date Légifrance (millisecondes, secondes, ou chaîne ISO) → jour « AAAA-MM-JJ ».
 * Minuit à Paris tombe la veille à 22 h / 23 h UTC : on ajoute 12 h avant de prendre le jour UTC.
 * Accepte les dates antérieures à 1970 (millisecondes négatives : codes de 1804…).
 */
export function jourLegifrance(raw: unknown): string | null {
  if (raw === null || raw === undefined || raw === '') return null;
  let ms: number | null = null;
  const n = typeof raw === 'number' ? raw : (typeof raw === 'string' && /^-?\d+$/.test(raw.trim()) ? Number(raw) : NaN);
  if (!Number.isNaN(n)) {
    ms = Math.abs(n) < 100_000_000_000 ? n * 1000 : n; // secondes → millisecondes
  } else if (typeof raw === 'string') {
    const t = Date.parse(raw);
    ms = Number.isNaN(t) ? null : t;
  }
  if (ms === null) return null;
  const d = new Date(ms + 12 * 3600 * 1000);
  if (Number.isNaN(d.getTime())) return null;
  return d.toISOString().slice(0, 10);
}

/** Fin de version : `null` si absente ou « sans fin » (2999). */
export function finLegifrance(raw: unknown): string | null {
  const j = jourLegifrance(raw);
  if (!j) return null;
  return Number(j.slice(0, 4)) >= ANNEE_SANS_FIN ? null : j;
}

/** « 1985-10-27 » → « 27/10/1985 ». */
export function jourFr(iso: string): string {
  const [y, m, d] = iso.split('-');
  return y && m && d ? `${d}/${m}/${y}` : iso;
}

export interface BandeauDroitADate {
  /** `true` : la version affichée était en vigueur à la date des faits. */
  applicable: boolean;
  /** Texte du bandeau (sans balisage). */
  message: string;
}

/**
 * Bandeau « droit applicable au… » pour la version affichée d'un article.
 * `null` : pas de date des faits, ou dates de la version inconnues (on ne dit rien plutôt que de deviner).
 */
export function bandeauDroitADate(
  date: string | null,
  debutRaw: unknown,
  finRaw: unknown,
): BandeauDroitADate | null {
  if (!date) return null;
  const debut = jourLegifrance(debutRaw);
  if (!debut) return null;
  const fin = finLegifrance(finRaw);
  const periode = fin ? `du ${jourFr(debut)} au ${jourFr(fin)}` : `depuis le ${jourFr(debut)}`;
  const quand = jourFr(date);
  const dedans = debut <= date && (fin === null || date <= fin);
  return dedans
    ? { applicable: true, message: `Droit applicable au ${quand} : cette rédaction était en vigueur (${periode}).` }
    : {
        applicable: false,
        message: `Cette rédaction n'était pas celle en vigueur au ${quand} (elle l'était ${periode}). Choisissez la version applicable ci-dessous.`,
      };
}
