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
// Juridix — Badge de vérification d'une référence d'article (chantier 4)
// ============================================================================
import React, { createContext, useContext } from 'react';
import {
  CITE_HREF_PREFIX, ancrageBadge, citationBadge, citationTitle,
  type CitationReport,
} from '../utils/citations';

/** Rapport de vérification du message en cours de rendu (null = aucun). */
export const CitationContext = createContext<CitationReport | null>(null);

/** Index de vérification encodé dans l'ancre, ou null si ce n'est pas une ancre de citation. */
export function parseCiteHref(href?: string): number | null {
  if (!href || !href.startsWith(CITE_HREF_PREFIX)) return null;
  const n = Number(href.slice(CITE_HREF_PREFIX.length));
  return Number.isInteger(n) && n >= 0 ? n : null;
}

export function CitationBadge({ index, children }: { index: number; children?: React.ReactNode }) {
  const report = useContext(CitationContext);
  const v = report?.verifications[index];
  if (!v) return <>{children}</>; // rapport disparu : texte brut, sans lien mort

  const existence = citationBadge(v);
  const ancrage = ancrageBadge(v);
  const title = citationTitle(v);
  const body = (
    <>
      <span className="cite-ref">{children}</span>
      {existence.icon && <span className="cite-flag" aria-label={existence.label}>{existence.icon}</span>}
      {ancrage && <span className="cite-flag cite-flag--ancrage" aria-label={ancrage.label}>{ancrage.icon}</span>}
    </>
  );
  const cls = [
    'cite',
    `cite--${v.statut}`,
    `cite--${v.kind}`,
    ancrage ? 'cite--hors-sources' : '',
  ].filter(Boolean).join(' ');
  return v.url ? (
    <a className={cls} href={v.url} target="_blank" rel="noopener noreferrer" title={title}>{body}</a>
  ) : (
    <span className={cls} title={title}>{body}</span>
  );
}
