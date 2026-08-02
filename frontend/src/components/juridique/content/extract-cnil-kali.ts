// ============================================================================
// Juridix — Espace "Recherche juridique" — Extraction KALI / CNIL
// ============================================================================
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { stripHtml, toHtmlString, sanitizeHtml, metaHtml, renderArticlesList, renderSections } from './html-helpers';
import type { ViewedContent } from './types';

/** Convention collective KALI — /consult/kaliText, /consult/kaliArticle
 *  (ConsultKaliTextResponse, schéma plat). */
export function extractKaliContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const conv = obj.convention as Record<string, unknown> | undefined ?? obj;
  const titreKali = stripHtml(toHtmlString(conv.titre || conv.title || titre));
  const idcc = stripHtml(toHtmlString(conv.idcc));
  const etat = stripHtml(toHtmlString(conv.etat));
  const meta = metaHtml([['IDCC', idcc || undefined], ['État', etat || undefined]]);
  const arts  = (conv.articles ?? conv.listArticle ?? []) as unknown[];
  const sects = (conv.sections ?? []) as unknown[];
  const bodyHtml = arts.length ? renderArticlesList(arts) : sects.length ? renderSections(sects) : '';
  return {
    format: 'html',
    body: [`<h1 class="jur-doc-h1">${titreKali}</h1>`, meta, bodyHtml || '<p class="jur-muted">Contenu non disponible.</p>'].filter(Boolean).join('\n'),
  };
}

/** CNIL — /consult/cnil (ConsultCnilTextResponse.text : TexteSimple — même
 *  enveloppe que /consult/juri, confirmé par le swagger officiel). */
export function extractCnilContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const d = (obj.text ?? obj) as Record<string, unknown>;
  const titreCnil = stripHtml(toHtmlString(d.titre || titre));
  const nor  = stripHtml(toHtmlString(d.nor));
  const num  = stripHtml(toHtmlString(d.num));
  const date_ = stripHtml(toHtmlString(d.dateTexte || d.datePubli));
  const html_ = toHtmlString(d.texteHtml || d.texte);
  const meta = metaHtml([['NOR', nor || undefined], ['N°', num || undefined], ['Date', date_ ? date_.slice(0, 10) : undefined]]);
  return { format: 'html', body: [`<h1 class="jur-doc-h1">${titreCnil}</h1>`, meta, html_ ? sanitizeHtml(html_) : '<p class="jur-muted">Texte non disponible.</p>'].filter(Boolean).join('\n') };
}
