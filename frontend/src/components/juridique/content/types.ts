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
// Type de retour commun à tous les extracteurs de content/extract-*.ts
// (extractArticleContent, extractCodeContent, extractLodaContent,
// extractJuriContent, extractKaliContent, extractCnilContent,
// extractCircContent, extractAccoContent, extractBoccContent,
// extractDebatContent, extractQuestionEcriteContent, extractDocAdminContent,
// extractDecisionContent) — chacun normalise la réponse brute de l'API
// (Légifrance ou Judilibre, schémas hétérogènes) vers cette forme unique,
// consommée telle quelle par JuridiquePanel.tsx (`viewedDoc.content.body`,
// injecté via dangerouslySetInnerHTML) et par useDocumentViewer.ts (dérivation
// de `contenuLlm` via stripHtml(content.body)).
// ============================================================================

export type ContentFormat = 'html' | 'markdown';

export interface ViewedContent {
  format: ContentFormat;
  body: string;
}
