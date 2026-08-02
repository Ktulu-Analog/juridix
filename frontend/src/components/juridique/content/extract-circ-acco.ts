// ============================================================================
// Juridix — Espace "Recherche juridique" — Extraction Circulaires / Accords
// ============================================================================
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import {
  stripHtml, toHtmlString, sanitizeHtml, metaHtml, formatLegiDate,
  formatPlainTextAsHtml, resolvePisteAssetUrl, resolveCircPdfUrl,
} from './html-helpers';
import type { ViewedContent } from './types';

/** Circulaire — ConsultCirculaireResponse.circulaire : Circulaire — schéma
 *  officiel confirmé : aucun champ de texte intégral, seulement un résumé/
 *  nota et un fichier PDF joint (le texte intégral, quand disponible, vient
 *  de `attachment.content` — extraction texte du PDF joint). */
export function extractCircContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const d = (obj.circulaire ?? obj) as Record<string, unknown>;
  const titreCirc = stripHtml(toHtmlString(d.titre || titre));
  const nor  = stripHtml(toHtmlString(d.nor));
  const numeroInterne = stripHtml(toHtmlString(d.numeroInterne));
  // dateSignature (et les autres dates de ce fonds) sont en EPOCH
  // SECONDES (ex. 1423526400 → 10/02/2015), contrairement à la plupart des
  // autres fonds qui renvoient des millisecondes — formatLegiDate détecte
  // désormais les deux automatiquement.
  const dateSignature = formatLegiDate(d.dateSignature);
  const dateMiseEnLigne = formatLegiDate(d.dateDepot || d.dateExport);
  const dateApplication = formatLegiDate(d.dateMiseApplication);
  const auteur = stripHtml(toHtmlString(d.auteur));
  const signataire = stripHtml(toHtmlString(d.signataire));
  const destinataire = stripHtml(toHtmlString(d.destinataire));
  const domaines = Array.isArray(d.domaines) ? (d.domaines as unknown[]).map((v) => stripHtml(toHtmlString(v))).filter(Boolean).join(', ') : '';
  const ministeres = Array.isArray(d.ministeresDeposants) ? (d.ministeresDeposants as unknown[]).map((v) => stripHtml(toHtmlString(v))).filter(Boolean).join(', ') : '';
  const categories = Array.isArray(d.categories) ? (d.categories as unknown[]).map((v) => stripHtml(toHtmlString(v))).filter(Boolean).join(', ') : '';
  const motsCles = Array.isArray(d.motsCles) ? (d.motsCles as unknown[]).map((v) => stripHtml(toHtmlString(v))).filter(Boolean) : [];
  const refPublication = stripHtml(toHtmlString(d.referencePublicationJoBo));
  const remplace = stripHtml(toHtmlString(d.remplace));
  const textesRef = Array.isArray(d.textesReferences) ? (d.textesReferences as Array<Record<string, unknown>>) : [];
  const resume_ = toHtmlString(d.resume);
  const notaHtml_ = toHtmlString(d.notaHtml || d.nota);
  // Texte intégral extrait du fichier joint, quand l'API le fournit
  // (Attachment.content) — sinon la circulaire reste PDF-only.
  const attachment = (d.attachment && typeof d.attachment === 'object') ? d.attachment as Record<string, unknown> : {};
  const attachmentText = toHtmlString(attachment.content);
  const pdfUrl = resolveCircPdfUrl(d) || resolvePisteAssetUrl(d.attachmentUrl);
  const pdfSize = stripHtml(toHtmlString(d.sizeToDisplay));
  const meta = metaHtml([
    ['Domaine(s)', domaines || undefined],
    ['Signature', dateSignature || undefined],
    ['Mise en ligne', dateMiseEnLigne || undefined],
    ['Mise en application', dateApplication || undefined],
    ['Ministère(s) déposant(s)', ministeres || undefined],
    ['NOR', nor || undefined],
    ['Numéro interne', numeroInterne || undefined],
    ['Référence de publication', refPublication || undefined],
    ['Auteur', auteur || undefined],
    ['Destinataire(s)', destinataire || undefined],
    ['Signataire', signataire || undefined],
    ['Catégorie(s)', categories || undefined],
  ]);
  const motsClesHtml = motsCles.length
    ? `<p class="jur-themes"><strong>Mots-clés :</strong> ${motsCles.join(' · ')}</p>` : '';
  const textesRefHtml = textesRef.length
    ? `<section class="jur-section"><h2>Texte(s) de référence</h2><ul>${
        textesRef.map((t) => `<li>${stripHtml(toHtmlString(t.texteReference))}</li>`).join('')
      }</ul></section>` : '';
  const remplaceHtml = remplace
    ? `<section class="jur-section"><h2>Circulaires qui ne sont plus applicables</h2><p>${sanitizeHtml(remplace)}</p></section>` : '';
  const pdfBlock = pdfUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${pdfUrl}" target="_blank" rel="noopener noreferrer">⬇ Télécharger la circulaire (PDF${pdfSize ? ` — ${pdfSize}` : ''})</a></p>`
    : `<p class="jur-muted">⚠️ Aucun fichier disponible pour cette circulaire.</p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreCirc}</h1>`, meta, motsClesHtml,
      resume_ ? `<section class="jur-section"><h2>Résumé</h2>${sanitizeHtml(resume_)}</section>` : '',
      attachmentText ? `<section class="jur-section jur-section--decision-text"><h2>Texte</h2>${formatPlainTextAsHtml(attachmentText)}</section>` : '',
      notaHtml_ ? `<section class="jur-section"><h2>Nota</h2>${sanitizeHtml(notaHtml_)}</section>` : '',
      textesRefHtml, remplaceHtml,
      !resume_ && !attachmentText && !notaHtml_ ? `<p class="jur-muted">Les circulaires sont diffusées en PDF ; l'API ne fournit pas systématiquement de texte intégral structuré pour ce fonds.</p>` : '',
      pdfBlock,
    ].filter(Boolean).join('\n'),
  };
}

/** Accord d'entreprise (ACCO) — ConsultAccoResponse.acco : Accord — schéma
 *  officiel confirmé : aucun champ de texte intégral, seulement des
 *  métadonnées et un fichier attaché. */
export function extractAccoContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  const d = (obj.acco ?? obj) as Record<string, unknown>;
  const titreAcco = stripHtml(toHtmlString(d.titreTexte || titre));
  const siret  = stripHtml(toHtmlString(d.siret));
  const raison = stripHtml(toHtmlString(d.raisonSociale));
  const date_  = stripHtml(toHtmlString(d.dateDepot || d.dateTexte));
  const themes_ = Array.isArray(d.themes) ? (d.themes as unknown[]).map((t) => stripHtml(toHtmlString(t))).filter(Boolean).join(', ') : '';
  const fileUrl = resolvePisteAssetUrl(d.attachementUrl || d.url);
  const meta = metaHtml([['Entreprise', raison || undefined], ['SIRET', siret || undefined], ['Thème(s)', themes_ || undefined], ['Dépôt', date_ ? date_.slice(0, 10) : undefined]]);
  const fileBlock = fileUrl
    ? `<p class="jur-pdf-download"><a class="jur-btn jur-btn--primary" href="${fileUrl}" target="_blank" rel="noopener noreferrer">⬇ Télécharger l'accord</a></p>`
    : `<p class="jur-muted">⚠️ Aucun fichier disponible pour cet accord.</p>`;
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreAcco}</h1>`, meta,
      `<p class="jur-muted">Les accords d'entreprise sont diffusés sous forme de fichier ; l'API ne fournit pas de texte intégral structuré pour ce fonds.</p>`,
      fileBlock,
    ].filter(Boolean).join('\n'),
  };
}
