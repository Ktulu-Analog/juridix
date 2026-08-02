// ============================================================================
// Juridix — Espace "Recherche juridique" — Helpers de recherche/résultats
// ============================================================================
// Fonctions pures (aucun état) utilisées à la fois par useJuridiqueSearch,
// useDocumentViewer et le JSX resté dans JuridiquePanel.tsx (liste de
// résultats, badges…) — d'où leur isolement dans un module à part plutôt
// que dans l'un ou l'autre hook.
//
// Extrait de JuridiquePanel.tsx (étape 5 du refactor — cf. discussion du
// 05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { stripHtml, toHtmlString, formatLegiDate } from './content/html-helpers';
import { FONDS_SANS_RECHERCHE_TEXTUELLE } from './constants';
import type { Source, Fond, SearchResult } from './types';

// BOCC a une recherche textuelle optionnelle (filtre `titre`, sous-chaîne) :
// ni obligatoire (contrairement aux fonds standards), ni absente (contrairement
// aux 3 fonds ci-dessus). Fonction unique utilisée à la fois par le bouton de
// recherche et par `doSearch`, pour garantir qu'ils restent cohérents.
export function fondRequiresQuery(source: Source, fond: Fond): boolean {
  if (source === 'judilibre') return true;
  if (FONDS_SANS_RECHERCHE_TEXTUELLE.includes(fond)) return false;
  if (fond === 'BOCC') return false;
  return true;
}

/** Formate une date Légifrance — accepte timestamp ms (number ou string),
 *  ISO 8601 avec ou sans timezone, ou YYYY-MM-DD.
 *  Robuste face aux engines WebKit/Tauri qui rejettent certains formats. */
export function extractTitle(r: SearchResult, src: Source = 'legifrance'): string {
  // ── JUDILIBRE ─────────────────────────────────────────────────────────────
  if (src === 'judilibre') {
    const jurisdiction = (r.jurisdiction as string) || '';
    const chamber      = (r.chamber      as string) || '';
    const date         = (r.decision_date as string) || (r.decision_datetime as string) || '';
    const num          = Array.isArray(r.numbers) && r.numbers.length ? r.numbers[0] : (r.num as string) || '';
    const solution     = (r.solution     as string) || '';
    const dateStr = date
      ? new Date(date).toLocaleDateString('fr-FR', { day: '2-digit', month: 'long', year: 'numeric' })
      : '';
    const courtLine = [
      jurisdiction,
      chamber && chamber !== jurisdiction ? chamber : '',
    ].filter(Boolean).join(', ');
    const parts = [courtLine, dateStr].filter(Boolean).join(' — ');
    const suffix = [num && `Pourvoi n° ${num}`, solution].filter(Boolean).join(' · ');
    if (parts) return suffix ? `${parts}\n${suffix}` : parts;
    if (num) return `Pourvoi n° ${num}`;
    return r.id || 'Décision';
  }

  // ── LÉGIFRANCE — strip les balises <mark> des highlights ─────────────────
  if (r.titles?.length) return stripHtml(r.titles[0].title || 'Sans titre');
  if (r.title)           return stripHtml(r.title);
  if (r.num)             return `N° ${r.num}`;
  if (r.nor)             return `NOR ${r.nor}`;
  return r.id || 'Document sans titre';
}

/**
 * Renvoie l'identifiant exploitable pour ouvrir le DOCUMENT pointé par un
 * résultat de recherche — PAS l'identifiant du texte qui le contient.
 *
 * D'après le swagger officiel Légifrance (definitions SearchResult /
 * SearchTitle / SearchSection / SearchExtract) :
 *   - `titles[].id` et `titles[].cid` identifient le TEXTE entier
 *     (LEGITEXT/JORFTEXT) → valable pour /consult/code ou /consult/legiPart,
 *     jamais pour /consult/getArticle.
 *   - `sections[].extracts[].id` identifie le FRAGMENT effectivement trouvé
 *     par la recherche (article LEGIARTI/JORFARTI, section JORFSCTA…) →
 *     c'est la seule source fiable pour /consult/getArticle.
 *
 * Pour les fonds CODE_*, on veut la table des matières du code, donc l'id de
 * texte (titles[].cid) reste pertinent. Pour tous les autres fonds (LODA,
 * JORF…), on doit prioriser l'extract le plus profond.
 */
export function extractResultId(r: SearchResult, isCodeFond: boolean): string {
  if (r.id) return r.id;

  if (!isCodeFond) {
    // Cherche l'identifiant d'article/section dans sections[].extracts[].id
    for (const section of r.sections || []) {
      for (const extract of section.extracts || []) {
        if (extract.id) return extract.id;
      }
      if (section.id) return section.id;
    }
  }

  // Fallback : id du texte entier (titres) — uniquement pertinent pour les
  // codes ou en dernier recours si aucun extract n'a été trouvé.
  return r.titles?.[0]?.cid || r.titles?.[0]?.id || '';
}

export function extractDate(r: SearchResult): string {
  const raw = r.dateSignature || r.datePublication || r.date || '';
  return formatLegiDate(raw);
}

/**
 * LÉGIFRANCE — extrait le premier snippet de texte surligné trouvé dans
 * `sections[].extracts[].values` (balisé en <mark> par l'API PISTE). C'est
 * l'équivalent fonctionnel des `highlights` Judilibre : un aperçu du passage
 * où les termes recherchés apparaissent dans le document.
 * Renvoie `null` si aucun extrait n'est disponible (cas fréquent pour les
 * fonds qui ne renvoient que des `titles[].title` surlignés, sans extracts).
 */
export function extractLegifranceHighlight(r: SearchResult): string | null {
  for (const section of r.sections || []) {
    for (const extract of section.extracts || []) {
      const v = extract.values?.[0];
      if (v) return v;
    }
  }
  return null;
}

/** Retourne le modificateur CSS (couleur sémantique) du badge solution. */
export function solutionModifier(sol: string): string {
  const s = sol.toLowerCase();
  if (s.includes('cassation'))   return 'cassation';
  if (s.includes('rejet'))       return 'rejet';
  if (s.includes('annulation'))  return 'annulation';
  if (s.includes('irrecevab'))   return 'irrecevabilite';
  if (s.includes('qpc'))         return 'qpc';
  return 'default';
}

/**
 * Normalisation des 4 fonds "spéciaux" (BOCC, débats parlementaires,
 * questions écrites parlementaires, documents administratifs).
 *
 * Ces fonds ne passent PAS par `/search` (ils n'existent pas dans l'enum
 * `SearchRequestDTO.fond` du swagger officiel) : ils n'ont que des
 * points d'entrée `/list/*` dédiés, sans capacité de recherche plein texte
 * pour 3 d'entre eux (seul BOCC accepte un filtre `titre` en sous-chaîne).
 * Leurs objets n'ont pas non plus la forme `SearchResult` standard (pas de
 * `titles[]`, parfois pas de `title` du tout) — on synthétise donc `id`,
 * `title` et `date` ici pour que `extractTitle`/`extractDate`/
 * `extractResultId` fonctionnent sans modification, tout en conservant tous
 * les champs d'origine (nécessaires au rendu fidèle dans extractTextContent
 * et pour permettre l'ouverture du document sans second appel réseau — voir
 * `openDocument`).
 */
export function normalizeBoccTextResult(t: Record<string, unknown>): SearchResult {
  const title = stripHtml(toHtmlString(t.title || t.enteteTitle)) || 'Texte BOCC';
  const id = (t.fileName as string) || (t.pathFile as string) || '';
  return { ...t, id, title, date: (t.texteDate as string) || undefined } as SearchResult;
}

export function normalizeDebatResult(d: Record<string, unknown>): SearchResult {
  const typeAssemblee = stripHtml(toHtmlString(d.typeAssemblee));
  const dateSeance = (d.dateSeance as string) || (d.dateParution as string) || '';
  const dateLabel = dateSeance ? formatLegiDate(dateSeance) : '';
  const title = [typeAssemblee, dateLabel].filter(Boolean).join(' — ') || 'Débat parlementaire';
  // Même précaution que pour QUESTION_ECRITE/DOC_ADMIN ci-dessous : `id` peut
  // être vide côté données réelles PISTE malgré le typage `string` du schéma
  // officiel `Debat`. Repli sur les autres identifiants toujours renseignés.
  const id = (d.id as string)
    || (d.idTechInjection as string)
    || (d.refInjection as string)
    || (d.pathToFile as string)
    || '';
  return { ...d, id, title, date: dateSeance || undefined } as SearchResult;
}

export function normalizeQuestionResult(q: Record<string, unknown>): SearchResult {
  const emetteur = stripHtml(toHtmlString(q.emetteur));
  const dateParution = (q.dateParution as string) || '';
  const dateLabel = dateParution ? formatLegiDate(dateParution) : '';
  const title = [emetteur, dateLabel].filter(Boolean).join(' — ') || 'Question écrite parlementaire';
  // `id` est vide en pratique sur une partie des résultats renvoyés par
  // `/list/questionsEcritesParlementaires` (même défaut de données que celui
  // déjà corrigé pour BOCC ci-dessus, cf. normalizeBoccTextResult) — malgré
  // le typage `string` du swagger officiel (EsQuestionsEcritesParlementaires),
  // qui ne garantit pas la non-nullité du champ côté données réelles PISTE.
  // Repli sur les autres identifiants du schéma, toujours renseignés :
  // `idTechInjection`/`refInjection` (identifiants techniques d'injection),
  // puis `pdfName`/`pathToFile` (nom du fichier PDF source, comme pour BOCC).
  const id = (q.id as string)
    || (q.idTechInjection as string)
    || (q.refInjection as string)
    || (q.pdfName as string)
    || (q.pathToFile as string)
    || '';
  return { ...q, id, title, date: dateParution || undefined } as SearchResult;
}

export function normalizeDocAdminResult(a: Record<string, unknown>): SearchResult {
  const title = stripHtml(toHtmlString(a.titre)) || 'Document administratif';
  // Même précaution que pour QUESTION_ECRITE ci-dessus : repli sur un
  // identifiant technique si `id` est vide (champs confirmés par le schéma
  // officiel DocumentAdministratif : idTechInjection, refInjection,
  // attachmentName — pas de champ `cid` sur ce schéma).
  const id = (a.id as string)
    || (a.idTechInjection as string)
    || (a.refInjection as string)
    || (a.attachmentName as string)
    || '';
  return { ...a, id, title, date: (a.dateDocument as string) || undefined } as SearchResult;
}
