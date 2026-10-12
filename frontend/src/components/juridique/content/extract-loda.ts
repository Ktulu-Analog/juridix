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
// Extraction LODA / CODE / article
// ============================================================================
// Fonds concernés : LODA_ETAT, LODA_DATE, JORF, CODE_ETAT, CODE_DATE, et les
// articles individuels (LEGIARTI, JORFARTI, KALIARTI...).
// Schéma officiel : ConsultTextResponse (/consult/legiPart, /consult/code/
// tableMatieres) et GetArticleResponse (/consult/getArticle).
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import {
  stripHtml, toHtmlString, sanitizeHtml, metaHtml, formatLegiDate,
  resolvePisteAssetUrl, linkifyLegifranceUrls, injectCitationLinks,
  renderArticleVersionsBlock, renderArticlesList, renderSections,
  renderArticleHistoriqueSection, renderArticleModificationsSection,
} from './html-helpers';
import type { ViewedContent } from './types';

/** Texte court d'un article cité, destiné à être inséré INLINE sous un lien
 *  de citation (cf. `resolveInlineCitations` dans html-helpers.ts) — pas
 *  d'affichage de titre/meta/breadcrumb/historique, seulement le texte de
 *  l'article et son nota éventuel, comme un extrait déjà cité. Retourne null
 *  si l'article n'a pas de contenu exploitable (dé-référencé, non publié…).
 */
export function extractArticleInlineText(obj: Record<string, unknown>): string | null {
  const article = obj.article as Record<string, unknown> | null | undefined;
  if (!article || Object.keys(article).length === 0) return null;
  const texteHtml = toHtmlString(article.texteHtml || article.texte);
  const notaHtml = toHtmlString(article.notaHtml || article.nota);
  const body = sanitizeHtml(linkifyLegifranceUrls(injectCitationLinks(texteHtml)));
  if (!stripHtml(body)) return null;
  const nota = notaHtml
    ? `<aside class="jur-nota"><strong>Nota</strong><br>${sanitizeHtml(notaHtml)}</aside>` : '';
  return [body, nota].filter(Boolean).join('\n');
}

/** Article individuel (LEGIARTI, JORFARTI, KALIARTI, etc.) —
 *  GetArticleResponse.article. */
export function extractArticleContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const article = obj.article as Record<string, unknown> | null | undefined;
  // L'API peut retourner article: null (document dé-référencé, non publié ou accès restreint)
  if (article === null || (article !== undefined && Object.keys(article).length === 0)) {
    const dereferenced = obj.dereferenced === true;
    const msg = dereferenced
      ? "Cet article a été dé-référencé et n'est plus accessible via l'API."
      : "Le contenu de cet article n'est pas disponible (accès restreint ou document non publié).";
    return {
      format: 'html',
      body: `<h1 class="jur-doc-h1">${stripHtml(titre)}</h1><p class="jur-muted jur-muted--warn">⚠️ ${msg}</p>`,
    };
  }
  if (article) {
    const texteHtml  = toHtmlString(article.texteHtml || article.texte);
    const notaHtml   = toHtmlString(article.notaHtml  || article.nota);
    const filAriane  = stripHtml(toHtmlString(article.fullSectionsTitre || article.sectionParentTitre));
    const num        = stripHtml(toHtmlString(article.num));
    const etat       = stripHtml(toHtmlString(article.etat));
    const meta = metaHtml([
      ['Article', num || undefined],
      ['État', etat || undefined],
    ]);
    const breadcrumb = filAriane
      ? `<blockquote class="jur-article-path">${filAriane}</blockquote>` : '';
    const id = stripHtml(toHtmlString(article.id));
    const versionsHtml = renderArticleVersionsBlock(article, id, /* standalone */ true);
    const body = sanitizeHtml(linkifyLegifranceUrls(injectCitationLinks(texteHtml)));
    const nota = notaHtml
      ? `<aside class="jur-nota"><strong>Nota</strong><br>${sanitizeHtml(notaHtml)}</aside>` : '';
    // A2 — champs déjà présents dans le payload de /consult/getArticle
    // (schéma Article), jamais rendus jusqu'ici. Le nom réel du champ de
    // liens de modification sur CET endpoint est `lienModifications` ; on
    // accepte aussi `lstLienModification` (nom utilisé par le schéma voisin
    // ConsultArticle, cf. note dans html-helpers.ts) au cas où l'API
    // renverrait un jour ce shape ici.
    const historiqueHtml = renderArticleHistoriqueSection(article.historique);
    const modificationsHtml = renderArticleModificationsSection(
      article.lienModifications ?? article.lstLienModification,
    );
    return {
      format: 'html',
      body: [
        `<h1 class="jur-doc-h1">${stripHtml(titre)}</h1>`,
        breadcrumb, meta, versionsHtml, body, nota,
        modificationsHtml, historiqueHtml,
      ].filter(Boolean).join('\n'),
    };
  }
  return { format: 'html', body: '' };
}

/** Code (LEGITEXT, structure arborescente) — table des matières via
 *  /consult/code/tableMatieres (ConsultTextResponse, plat). */
export function extractCodeContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const titreLoi = stripHtml(toHtmlString(obj.titre || obj.title || titre));
  const etat     = stripHtml(toHtmlString(obj.etat || obj.state));
  const dateText = stripHtml(toHtmlString(obj.dateTexte || obj.dateVersion));
  const meta = metaHtml([['État', etat || undefined], ['Date', dateText || undefined]]);
  const articles = (obj.articles ?? []) as unknown[];
  const sections = (obj.sections ?? obj.listArticle ?? []) as unknown[];
  const articlesHtml = (articles.length || sections.length)
    ? [articles.length ? renderArticlesList(articles) : '', sections.length ? renderSections(sections) : ''].filter(Boolean).join('\n')
    : `<p class="jur-muted">Structure non disponible — ouvrez un article individuellement.</p>`;
  return {
    format: 'html',
    body: [`<h1 class="jur-doc-h1">${titreLoi}</h1>`, meta, articlesHtml].filter(Boolean).join('\n'),
  };
}

/** Texte LODA (loi, décret, arrêté, ordonnance…) — /consult/legiPart,
 *  /consult/jorf-part (ConsultTextResponse, plat). */
export function extractLodaContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  // L'API Légifrance (ConsultTextResponse) retourne les champs à plat (title, nor, sections, articles…)
  // Mais certains anciens formats nichent tout sous 'texte'. On supporte les deux.
  const texte = (obj.texte as Record<string, unknown> | undefined) ?? obj;
  const titreLoda = stripHtml(toHtmlString(obj.title || obj.titre || texte.titre || texte.title || titre));
  const nor    = stripHtml(toHtmlString(obj.nor    || texte.nor));
  const num    = stripHtml(toHtmlString(obj.textNumber || texte.num || texte.numero));
  const etat   = stripHtml(toHtmlString(obj.etat   || texte.etat   || texte.state));
  const nature = stripHtml(toHtmlString(obj.nature || texte.nature));
  const dateSigRaw = obj.dateTexte || texte.dateSignature || texte.dateTexte;
  const dateJORaw  = obj.dateParution || texte.datePublication;
  const visa    = stripHtml(toHtmlString(obj.visa  || texte.visa));
  const meta = metaHtml([
    ['NOR', nor || undefined], ['N°', num || undefined],
    ['Nature', nature || undefined], ['État', etat || undefined],
    ['Signature', formatLegiDate(dateSigRaw) || undefined],
    ['Parution JO', formatLegiDate(dateJORaw) || undefined],
  ]);
  // Visa (visas du texte) affiché comme note introductive
  const visaHtml = visa
    ? `<blockquote class="jur-article-path">${visa}</blockquote>` : '';
  // Articles racine (ConsultTextResponse.articles) ET sections
  // (ConsultTextResponse.sections, ex. « Annexe ») : un même texte JORF
  // peut avoir les deux à la racine (Article 1/2/3 + une section Annexe).
  // L'ancien code ne rendait que l'un OU l'autre (le premier non vide),
  // ce qui faisait disparaître silencieusement les annexes dès qu'il y
  // avait aussi des articles racine.
  const articles = (obj.articles ?? texte.articles ?? obj.listArticle ?? []) as unknown[];
  const sections_ = (obj.sections ?? texte.sections ?? []) as unknown[];
  const bodyHtml = [
    articles.length ? renderArticlesList(articles) : '',
    sections_.length ? renderSections(sections_) : '',
  ].filter(Boolean).join('\n') || '<p class="jur-muted">Contenu non disponible.</p>';
  // Extrait PDF du Journal officiel électronique authentifié
  // (ConsultTextResponse.filePath/fileName/fileSize).
  const pdfPath = obj.filePath || texte.filePath;
  const pdfUrl = resolvePisteAssetUrl(pdfPath);
  const pdfSize = stripHtml(toHtmlString(obj.fileSize || texte.fileSize));
  const pdfBlock = pdfUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${pdfUrl}" target="_blank" rel="noopener noreferrer">⬇ Extrait du Journal officiel électronique authentifié (PDF${pdfSize ? ` — ${pdfSize}` : ''})</a></p>`
    : '';
  return {
    format: 'html',
    body: [`<h1 class="jur-doc-h1">${titreLoda}</h1>`, meta, visaHtml, pdfBlock, bodyHtml].filter(Boolean).join('\n'),
  };
}
