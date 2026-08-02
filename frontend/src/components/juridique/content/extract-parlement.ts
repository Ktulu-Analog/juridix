// ============================================================================
// Juridix — Espace "Recherche juridique" — Extraction fonds "PDF-only"
// ============================================================================
// Fonds concernés : BOCC, débats parlementaires, questions écrites
// parlementaires, documents administratifs. Point commun confirmé par le
// swagger officiel (schémas EsTextBocc, Debat, EsQuestionsEcritesParlementaires,
// DocumentAdministratif) : AUCUN champ de contenu texte/HTML — uniquement des
// métadonnées et une référence de fichier PDF (parfois XML).
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import {
  stripHtml, toHtmlString, metaHtml, formatLegiDate, formatPlainTextAsHtml,
  resolvePisteAssetUrl, resolveDebatPdfUrl, resolveQuestionEcritePdfUrl,
} from './html-helpers';
import type { ViewedContent } from './types';

/** BOCC — texte unitaire d'un bulletin officiel des conventions collectives
 *  (schéma officiel EsTextBocc : aucun champ de contenu texte, uniquement
 *  une référence de fichier PDF). */
export function extractBoccContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const titreBocc = stripHtml(toHtmlString(obj.title || obj.enteteTitle || titre));
  const idccs = Array.isArray(obj.idccs) ? (obj.idccs as string[]).join(', ') : stripHtml(toHtmlString(obj.idccs));
  const dept  = stripHtml(toHtmlString(obj.department));
  const date_ = formatLegiDate(obj.texteDate || obj.date);
  const taille = stripHtml(toHtmlString(obj.displaySize));
  const idMain = stripHtml(toHtmlString(obj.idMainBocc));
  const pdfUrl = resolvePisteAssetUrl(obj.pathFile || obj.pathToFile);
  const meta = metaHtml([
    ['IDCC', idccs || undefined],
    ['Émetteur', dept || undefined],
    ['Bulletin', idMain || undefined],
    ['Date', date_ || undefined],
    ['Taille', taille || undefined],
  ]);
  const pdfBlock = pdfUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${pdfUrl}" target="_blank" rel="noopener noreferrer">⬇ Télécharger le PDF</a></p>`
    : `<p class="jur-muted">⚠️ Aucun fichier disponible pour ce texte.</p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreBocc}</h1>`, meta,
      `<p class="jur-muted">Ce fonds (Bulletin officiel des conventions collectives) est constitué de fichiers PDF ; l'API ne fournit pas de contenu textuel structuré pour ce type de document.</p>`,
      pdfBlock,
    ].filter(Boolean).join('\n'),
  };
}

/** Débat parlementaire — ConsultDebatResponse.debat : Debat — schéma
 *  officiel confirmé : aucun champ de contenu texte hors le champ libre
 *  `data` parfois renseigné — le reste est un fichier PDF. */
export function extractDebatContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const d = (obj.debat ?? obj) as Record<string, unknown>;
  const typeAssemblee = stripHtml(toHtmlString(d.typeAssemblee));
  const session = stripHtml(toHtmlString(d.nomSession || d.session));
  const legislature = stripHtml(toHtmlString(d.legislature));
  const dateSeance = formatLegiDate(d.dateSeance);
  const dateParution = formatLegiDate(d.dateParution);
  const numParution = stripHtml(toHtmlString(d.numeroParution));
  const taille = stripHtml(toHtmlString(d.displaySize));
  const titreDebat = stripHtml(toHtmlString(
    titre || [typeAssemblee, dateSeance].filter(Boolean).join(' — ') || 'Débat parlementaire'
  ));
  const donnee = toHtmlString(d.data);
  const pdfUrl = resolveDebatPdfUrl(d) || resolvePisteAssetUrl(d.attachmentUrl);
  const meta = metaHtml([
    ['Assemblée', typeAssemblee || undefined],
    ['Législature', legislature || undefined],
    ['Session', session || undefined],
    ['Séance du', dateSeance || undefined],
    ['Parution', dateParution || undefined],
    ['N° parution', numParution || undefined],
    ['Taille', taille || undefined],
  ]);
  const pdfBlock = pdfUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${pdfUrl}" target="_blank" rel="noopener noreferrer">⬇ Télécharger le compte-rendu (PDF)</a></p>`
    : `<p class="jur-muted">⚠️ Aucun fichier disponible pour ce débat.</p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreDebat}</h1>`, meta,
      donnee ? `<section class="jur-section"><h2>Contenu</h2>${formatPlainTextAsHtml(donnee)}</section>` : '',
      !donnee ? `<p class="jur-muted">Les débats parlementaires sont diffusés en PDF (compte-rendu intégral) ; l'API ne fournit pas systématiquement de contenu textuel structuré.</p>` : '',
      pdfBlock,
    ].filter(Boolean).join('\n'),
  };
}

/** Question écrite parlementaire — schéma officiel
 *  EsQuestionsEcritesParlementaires : aucun champ de contenu texte,
 *  uniquement une référence de fichier PDF. */
export function extractQuestionEcriteContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const emetteur = stripHtml(toHtmlString(obj.emetteur));
  const origine = stripHtml(toHtmlString(obj.origine));
  const numParution = stripHtml(toHtmlString(obj.numeroParution));
  const dateParution = formatLegiDate(obj.dateParution);
  const taille = stripHtml(toHtmlString(obj.displaySize));
  const titreQ = stripHtml(toHtmlString(
    titre || [emetteur, dateParution].filter(Boolean).join(' — ') || 'Question écrite parlementaire'
  ));
  const pdfUrl = resolveQuestionEcritePdfUrl(obj) || resolvePisteAssetUrl(obj.pathToFile);
  const meta = metaHtml([
    ['Émetteur', emetteur || undefined],
    ['Origine', origine || undefined],
    ['N° parution', numParution || undefined],
    ['Date', dateParution || undefined],
    ['Taille', taille || undefined],
  ]);
  const pdfBlock = pdfUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${pdfUrl}" target="_blank" rel="noopener noreferrer">⬇ Télécharger le PDF</a></p>`
    : `<p class="jur-muted">⚠️ Aucun fichier disponible pour cette question.</p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreQ}</h1>`, meta,
      `<p class="jur-muted">Les questions écrites parlementaires sont diffusées en PDF ; l'API ne fournit pas de contenu textuel structuré (question/réponse) pour ce fonds.</p>`,
      pdfBlock,
    ].filter(Boolean).join('\n'),
  };
}

/** Document administratif — schéma officiel DocumentAdministratif : aucun
 *  champ de contenu texte — fichier PDF signé + fichier XML.
 *
 * ⚠️ `attachmentUrl` ("chemin vers le fichier attaché") NE PERMET PAS de
 * reconstruire un lien de téléchargement valide. Vérifié le 08/07/2026 en
 * comparant à la vraie page publique (https://www.legifrance.gouv.fr/liste/docAdmin) :
 * le lien de téléchargement réel y est de la forme
 *   https://www.legifrance.gouv.fr/download/pdf?id=<jeton opaque signé>
 * (ex. `id=1Kz63U-YWPD1fM1ao0ov4VsDFihSq-tW46KWa2ISZzs=`) — un jeton généré
 * côté serveur SANS RAPPORT avec `attachmentUrl`, `attachmentName`, `id` ou
 * tout autre champ renvoyé par l'API (contrairement aux débats parlementaires,
 * où `pathToFile` suffit à reconstruire le lien réel — cf. resolveDebatPdfUrl
 * ci-dessus, confirmé toujours exact le 08/07/2026 contre la page publique).
 * Le swagger officiel confirme par ailleurs qu'aucun endpoint PISTE n'expose
 * ce fichier : le téléchargement direct n'est donc pas reconstructible.
 *
 * ⚠️ CORRECTIF (08/07/2026) : le repli précédent (recherche Légifrance
 * préfiltrée par NOR, `resolveDocAdminSearchUrl`) s'est avéré pointer vers
 * le MAUVAIS document. Un document administratif de ce fonds est très
 * souvent l'ANNEXE d'un arrêté (tableaux, barèmes…), pas l'arrêté lui-même —
 * confirmé par observation de nombreux arrêtés publics dont le corps
 * contient systématiquement une clause du type « les annexes sont
 * consultables aux Documents administratifs (legifrance.gouv.fr/liste/
 * docAdmin) », l'arrêté et son annexe étant deux publications distinctes
 * partageant pourtant le même NOR. Rechercher par NOR sur `/search/all`
 * ramène donc le texte JORF de l'arrêté — pas son annexe, qui n'est publiée
 * QUE dans ce fonds. Sans paramètre de filtrage documenté sur `/liste/
 * docAdmin` (aucune trace publique d'un tel paramètre malgré recherche), il
 * n'existe pas de moyen fiable de construire un lien qui pointe précisément
 * sur l'annexe recherchée. On assume donc cette limite plutôt que de
 * proposer un lien qui semble précis mais ramène le mauvais document : le
 * bouton principal renvoie vers la liste officielle (filtrable manuellement
 * par l'utilisateur, avec ses propres facettes), et un second lien, quand
 * le NOR est disponible, permet de retrouver le texte JORF d'origine — utile
 * pour le contexte (objet de l'annexe), pas pour l'annexe elle-même. */
export function extractDocAdminContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const titreDa = stripHtml(toHtmlString(obj.titre || obj.title || titre));
  const sousTitre = stripHtml(toHtmlString(obj.sousTitre));
  const nor = stripHtml(toHtmlString(obj.nor));
  const numero = stripHtml(toHtmlString(obj.numero));
  const nature_ = stripHtml(toHtmlString(obj.nature));
  const origine = stripHtml(toHtmlString(obj.origine));
  const dateDoc = formatLegiDate(obj.dateDocument);
  const dateJO = formatLegiDate(obj.dateJO);
  const taille = stripHtml(toHtmlString(obj.displaySize));
  const meta = metaHtml([
    ['NOR', nor || undefined], ['N°', numero || undefined],
    ['Nature', nature_ || undefined], ['Origine', origine || undefined],
    ['Date du document', dateDoc || undefined], ['Date JO', dateJO || undefined],
    ['Taille', taille || undefined],
  ]);
  const listeUrl = 'https://www.legifrance.gouv.fr/liste/docAdmin';
  // Lien vers le texte JORF d'origine (contexte uniquement — ne contient
  // PAS l'annexe elle-même, cf. commentaire ci-dessus) : recherche par NOR
  // sur le fonds JORF, seul fonds où ce document existe réellement.
  const jorfContextUrl = nor
    ? `https://www.legifrance.gouv.fr/search/all?${new URLSearchParams({
        tab_selection: 'all', searchField: 'ALL', query: nor, page: '1', init: 'true',
      }).toString()}`
    : null;
  const pdfBlock = `<p class="jur-pdf-download">
    <a class="jur-btn jur-btn--primary" href="${listeUrl}" target="_blank" rel="noopener noreferrer">Parcourir les documents administratifs ↗</a>${jorfContextUrl
      ? `\n    <a class="jur-btn" href="${jorfContextUrl}" target="_blank" rel="noopener noreferrer">Voir le texte JORF associé (sans l'annexe) ↗</a>`
      : ''}
  </p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreDa}</h1>`,
      sousTitre ? `<p class="jur-doc-subtitle">${sousTitre}</p>` : '',
      meta,
      `<p class="jur-muted">Les documents administratifs (souvent des annexes d'arrêtés — tableaux, barèmes…) sont diffusés en PDF signé, séparément du texte JORF qui les institue. L'API ne fournit ni contenu textuel structuré ni lien de téléchargement direct pour ce fonds, et aucun moyen fiable de filtrer la liste officielle vers ce document précis : utilisez la recherche manuelle ci-dessous (par titre, NOR ou date).</p>`,
      pdfBlock,
    ].filter(Boolean).join('\n'),
  };
}
