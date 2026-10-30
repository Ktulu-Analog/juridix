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
import React, { createContext, useContext, useEffect, useState } from 'react';
import {
  CITE_HREF_PREFIX, ancrageBadge, citationBadge, citationTitle, droitADateBadge, droitADateOf,
  fetchFiche, ficheLines,
  type CitationReport, type FicheDecision,
} from '../utils/citations';

/** Rapport de vérification du message en cours de rendu (null = aucun). */
export const CitationContext = createContext<CitationReport | null>(null);

/** Index de vérification encodé dans l'ancre, ou null si ce n'est pas une ancre de citation. */
export function parseCiteHref(href?: string): number | null {
  if (!href || !href.startsWith(CITE_HREF_PREFIX)) return null;
  const n = Number(href.slice(CITE_HREF_PREFIX.length));
  return Number.isInteger(n) && n >= 0 ? n : null;
}

/** Fiche d'une décision retrouvée : rapprochements fournis par les API, avec leur provenance. */
function FichePanel({ id }: { id: string }) {
  const [fiche, setFiche] = useState<FicheDecision | null | undefined>(undefined); // undefined = chargement
  useEffect(() => {
    const ctrl = new AbortController();
    fetchFiche(id, ctrl.signal).then(f => { if (!ctrl.signal.aborted) setFiche(f); });
    return () => ctrl.abort();
  }, [id]);
  if (fiche === undefined) return <span className="cite-fiche">Chargement de la fiche…</span>;
  if (fiche === null) return <span className="cite-fiche">Fiche indisponible (source injoignable ou réponse inattendue).</span>;
  const { identite, rapprochements, limites } = ficheLines(fiche);
  return (
    <span className="cite-fiche">
      <strong>{identite[0]}</strong>
      {identite.slice(1).map(l => <span key={l} className="cite-fiche-ligne">{l}</span>)}
      <span className="cite-fiche-titre">Rapprochements fournis par l'API</span>
      {rapprochements.length === 0
        ? <span className="cite-fiche-ligne">Aucun rapprochement renseigné.</span>
        : rapprochements.map((l, i) => <span key={i} className="cite-fiche-ligne">• {l}</span>)}
      <span className="cite-fiche-titre">Non fournis par les API</span>
      {limites.map(l => <span key={l} className="cite-fiche-ligne cite-fiche-limite">• {l}</span>)}
    </span>
  );
}

export function CitationBadge({ index, children }: { index: number; children?: React.ReactNode }) {
  const report = useContext(CitationContext);
  const v = report?.verifications[index];
  if (!v) return <>{children}</>; // rapport disparu : texte brut, sans lien mort

  const existence = citationBadge(v);
  const ancrage = ancrageBadge(v);
  const title = citationTitle(v, report ?? null);
  const dad = droitADateOf(report ?? null, v);
  const temps = droitADateBadge(dad);
  const body = (
    <>
      <span className="cite-ref">{children}</span>
      {existence.icon && <span className="cite-flag" aria-label={existence.label}>{existence.icon}</span>}
      {ancrage && <span className="cite-flag cite-flag--ancrage" aria-label={ancrage.label}>{ancrage.icon}</span>}
      {temps && <span className="cite-flag cite-flag--temps" aria-label={temps.label}>{temps.icon}</span>}
    </>
  );
  const cls = [
    'cite',
    `cite--${v.statut}`,
    `cite--${v.kind}`,
    ancrage ? 'cite--hors-sources' : '',
  ].filter(Boolean).join(' ');
  // Avec une date des faits, le lien ouvre la version applicable à cette date.
  const href = dad?.statut === 'applicable' && dad.url_applicable ? dad.url_applicable : v.url;
  const ref = href ? (
    <a className={cls} href={href} target="_blank" rel="noopener noreferrer" title={title}>{body}</a>
  ) : (
    <span className={cls} title={title}>{body}</span>
  );
  // Décision retrouvée : accès à sa fiche (décision attaquée, suivante, rapprochements).
  if (v.kind !== 'decision' || !v.decision_id) return ref;
  return <DecisionWithFiche id={v.decision_id}>{ref}</DecisionWithFiche>;
}

function DecisionWithFiche({ id, children }: { id: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      {children}
      <button
        type="button"
        className="cite-fiche-btn"
        aria-expanded={open}
        aria-label="Rapprochements de jurisprudence de cette décision"
        title="Décision attaquée, décisions suivantes, rapprochements (avec leur provenance)"
        onClick={() => setOpen(o => !o)}
      >⛓</button>
      {open && <FichePanel id={id} />}
    </>
  );
}
