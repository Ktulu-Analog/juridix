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
// Point d'entrée unique appelé par JuridiquePanel.tsx : route vers le module
// d'extraction dédié à chaque fonds. Chaque module (extract-*.ts) porte les
// commentaires expliquant le schéma officiel utilisé et les pièges déjà
// rencontrés — voir ces fichiers avant de corriger un bug de mapping de
// champ pour un fonds donné.
// ============================================================================

import { stripHtml, toHtmlString, sanitizeHtml } from './html-helpers';
import { extractArticleContent, extractCodeContent, extractLodaContent } from './extract-loda';
import { extractJuriContent } from './extract-juri';
import { extractKaliContent, extractCnilContent } from './extract-cnil-kali';
import { extractCircContent, extractAccoContent } from './extract-circ-acco';
import {
  extractBoccContent, extractDebatContent, extractQuestionEcriteContent, extractDocAdminContent,
} from './extract-parlement';
import { extractDecisionContent } from './extract-judilibre';
import { extractDossierLegislatifContent } from './extract-dossier-legislatif';
import type { ViewedContent } from './types';

export type { ViewedContent, ContentFormat } from './types';

export function extractTextContent(raw: unknown, type: string, titre: string): ViewedContent {
  if (!raw || typeof raw !== 'object') return { format: 'html', body: '' };
  const obj = raw as Record<string, unknown>;

  if (type === 'article')          return extractArticleContent(obj, titre);
  if (type === 'decision')         return extractDecisionContent(obj);
  if (type === 'code')             return extractCodeContent(obj, titre);
  if (type === 'loda')             return extractLodaContent(obj, titre);
  if (type === 'juri')             return extractJuriContent(obj, titre);
  if (type === 'kali')             return extractKaliContent(obj, titre);
  if (type === 'cnil')             return extractCnilContent(obj, titre);
  if (type === 'circ')             return extractCircContent(obj, titre);
  if (type === 'acco')             return extractAccoContent(obj, titre);
  if (type === 'bocc')             return extractBoccContent(obj, titre);
  if (type === 'debat')            return extractDebatContent(obj, titre);
  if (type === 'question_ecrite')  return extractQuestionEcriteContent(obj, titre);
  if (type === 'doc_admin')        return extractDocAdminContent(obj, titre);
  if (type === 'dossier_legislatif') return extractDossierLegislatifContent(obj, titre);

  // Fallback — contenu non reconnu : afficher un message plutôt que du JSON brut
  // On tente d'extraire un texte ou titre de dernier recours
  const fallbackTitle = stripHtml(toHtmlString(
    (raw as Record<string,unknown>)?.title ||
    (raw as Record<string,unknown>)?.titre ||
    titre || ''
  ));
  const fallbackBody = toHtmlString(
    (raw as Record<string,unknown>)?.texteHtml ||
    (raw as Record<string,unknown>)?.texte ||
    (raw as Record<string,unknown>)?.content || ''
  );
  if (fallbackBody) {
    return {
      format: 'html',
      body: [
        fallbackTitle ? `<h1 class="jur-doc-h1">${fallbackTitle}</h1>` : '',
        sanitizeHtml(fallbackBody),
      ].filter(Boolean).join('\n'),
    };
  }
  return {
    format: 'html',
    body: [
      fallbackTitle ? `<h1 class="jur-doc-h1">${fallbackTitle}</h1>` : '',
      `<p class="jur-muted jur-muted--warn">⚠️ Le contenu de ce document n'est pas disponible ou son format n'est pas pris en charge.</p>`,
    ].filter(Boolean).join('\n'),
  };
}
