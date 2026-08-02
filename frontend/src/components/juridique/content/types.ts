// ============================================================================
// Juridix — Espace "Recherche juridique" — Types du contenu extrait
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
//
// NB : en pratique, tous les extracteurs produisent aujourd'hui du HTML
// ('html') — 'markdown' est conservé dans le type pour le cas, mentionné
// ailleurs dans le code, d'une décision Judilibre déjà en format Markdown
// natif (pas de citations HTML à résoudre) ; aucun extracteur ne l'utilise
// actuellement, mais le distinguo reste utile côté consommateur si ce
// format venait à être réintroduit.
//
// (Ce fichier ne définissait par erreur, avant correction, qu'une copie
// dupliquée de ../types.ts qui s'auto-important — cassant la compilation ;
// ViewedContent/ContentFormat n'étaient plus exportés du tout.)
//
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

export type ContentFormat = 'html' | 'markdown';

export interface ViewedContent {
  format: ContentFormat;
  body: string;
}
