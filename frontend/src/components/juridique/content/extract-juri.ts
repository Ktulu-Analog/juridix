// ============================================================================
// Juridix — Espace "Recherche juridique" — Extraction jurisprudence Légifrance
// ============================================================================
// Fonds concernés : JURI, CETAT, CONSTIT, JUFI — tous servis par le même
// endpoint /consult/juri (ConsultJuriTextResponse.text : TexteSimple).
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { stripHtml, toHtmlString, sanitizeHtml, metaHtml, injectCitationLinks } from './html-helpers';
import type { ViewedContent } from './types';

/** Jurisprudence JURI/CETAT/CONSTIT/JUFI (Légifrance).
 *
 * Schéma confirmé par le swagger officiel Légifrance (definitions
 * ConsultJuriTextResponse / TexteSimple, utilisé pour les 4 fonds
 * JURI/CETAT/CONSTIT/JUFI qui partagent tous /consult/juri) :
 *   ConsultJuriTextResponse.text : TexteSimple
 *   TexteSimple.{titre, juridiction, formation, solution, nature,
 *     numeroAffaire: string[], dateTexte, ecli, texteHtml, texte,
 *     sommaire: TexteSommaire[], titrages: string[], motsCles: string[]}
 *   TexteSommaire.{abstrats, resumePrincipal, autreResume}
 * L'ancien code lisait `decision.chambre` (n'existe pas — c'est
 * `formation`), `decision.numAffaire` en tant que string (c'est un
 * tableau nommé `numeroAffaire`), `decision.dateDecision` (c'est
 * `dateTexte`) et `decision.abstraits`/`sommaire` comme du HTML brut
 * (c'est un tableau d'objets) : ces désaccords de noms/formes
 * expliquaient l'affichage vide constaté pour la jurisprudence
 * administrative.
 */
export function extractJuriContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const decision = (obj.text ?? obj) as Record<string, unknown>;
  const titreDec    = stripHtml(toHtmlString(decision.titre || titre));
  const juridiction = stripHtml(toHtmlString(decision.juridiction || decision.natureJuridiction));
  const formation   = stripHtml(toHtmlString(decision.formation));
  const solution    = stripHtml(toHtmlString(decision.solution));
  const nature      = stripHtml(toHtmlString(decision.nature));
  const ecli        = stripHtml(toHtmlString(decision.ecli));
  const numeroAffaire = Array.isArray(decision.numeroAffaire)
    ? (decision.numeroAffaire as unknown[]).filter((n) => typeof n === 'string' && n.trim()).join(', ')
    : '';
  const dateDec_ = stripHtml(toHtmlString(decision.dateTexte || decision.datePubli || decision.relevantDate));
  const html_    = toHtmlString(decision.texteHtml || decision.texte);
  // `sommaire` est un tableau de {abstrats, resumePrincipal, autreResume},
  // pas une chaîne HTML — on assemble chaque entrée en paragraphes.
  const sommaireItems = Array.isArray(decision.sommaire)
    ? (decision.sommaire as Array<Record<string, unknown>>)
    : [];
  const sommaireHtml_ = sommaireItems
    .map((s) => [s.resumePrincipal, s.abstrats, s.autreResume]
      .filter((v): v is string => typeof v === 'string' && v.trim().length > 0)
      .map((v) => `<p>${sanitizeHtml(v)}</p>`).join(''))
    .filter(Boolean).join('');
  const titrages = Array.isArray(decision.titrages)
    ? (decision.titrages as unknown[]).filter((t) => typeof t === 'string' && t.trim()) as string[]
    : [];
  const titragesHtml = titrages.length
    ? `<section class="jur-section"><h2>Titrage</h2><ul>${titrages.map((t) => `<li>${stripHtml(t)}</li>`).join('')}</ul></section>` : '';
  const motsCles = Array.isArray(decision.motsCles)
    ? (decision.motsCles as unknown[]).filter((m) => typeof m === 'string' && m.trim()) as string[]
    : [];
  const motsClesHtml = motsCles.length
    ? `<p class="jur-themes"><strong>Mots-clés :</strong> ${motsCles.map(stripHtml).join(' · ')}</p>` : '';
  const meta = metaHtml([
    ['Juridiction', juridiction || undefined], ['Formation', formation || undefined],
    ['Nature', nature || undefined], ['Solution', solution || undefined],
    ['N° affaire', numeroAffaire || undefined], ['ECLI', ecli || undefined],
    ['Date', dateDec_ ? dateDec_.slice(0, 10) : undefined],
  ]);
  const hasContent = Boolean(html_ || sommaireHtml_ || titragesHtml);
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreDec}</h1>`, meta, motsClesHtml, titragesHtml,
      sommaireHtml_ ? `<section class="jur-section"><h2>Sommaire</h2>${sommaireHtml_}</section>` : '',
      html_ ? `<section class="jur-section jur-section--decision-text"><h2>Texte</h2>${sanitizeHtml(injectCitationLinks(html_))}</section>` : '',
      !hasContent ? `<p class="jur-muted jur-muted--warn">⚠️ Le texte de cette décision n'a pas pu être extrait de la réponse de l'API pour ce fonds. Consultez-la directement sur Légifrance ou signalez ce fonds via le bouton de retour d'information.</p>` : '',
    ].filter(Boolean).join('\n'),
  };
}
