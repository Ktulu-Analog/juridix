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
// Extraction Dossier législatif
// ============================================================================
// Fonds concerné : dossiers législatifs (id `JORFDOLE…`), travaux
// préparatoires d'une loi — exposé des motifs, arborescence des débats
// parlementaires par chambre (Sénat / Assemblée nationale), sous-dossiers
// avec leur propre contenu HTML.
//
// Schéma vérifié le 08/07/2026 contre le swagger officiel Légifrance
// v2.4.2 (définitions `ConsultDossierLegislatifResponse`, `DossierLegislatif`,
// `Dossier`, `Arborescence`, `Niveau`, `Lien`, `Legislature`) — cf.
// useDocumentViewer.ts pour le repli entre `raw.dossiersLegislatifs` (présent
// directement sur ConsultTextResponse/ConsultJorfResponse) et
// `SearchResult.dossiersLegislatifs`.//
// ============================================================================

import {
  stripHtml, toHtmlString, sanitizeHtml, metaHtml, formatLegiDate, linkifyLegifranceUrls,
} from './html-helpers';
import type { ViewedContent } from './types';
import type {
  DossierLegislatifRef, DossierLegislatifNiveau, DossierLegislatifLien, DossierLegislatifArborescence,
} from '../types';

/** Un lien terminal (feuille) de l'arborescence — toujours externe
 *  (Sénat/Assemblée nationale/JO), jamais un id Légifrance résoluble via
 *  data-cite-id, contrairement au reste de la navigation de l'application. */
function renderLien(lien: DossierLegislatifLien): string {
  const libelle = stripHtml(toHtmlString(lien.libelle)) || 'Étape';
  return lien.lien
    ? `<li><a href="${lien.lien}" target="_blank" rel="noopener noreferrer">${libelle}</a></li>`
    : `<li>${libelle}</li>`;
}

/** Nœud de l'arborescence — récursif (ex. « Sénat » contenant plusieurs
 *  liens et/ou sous-niveaux). Rendu en liste imbriquée. */
function renderNiveau(niveau: DossierLegislatifNiveau): string {
  const libelle = stripHtml(toHtmlString(niveau.libelle));
  const liens = (niveau.liens ?? []).map(renderLien).join('');
  const sousNiveaux = (niveau.niveaux ?? []).map(renderNiveau).join('');
  return `<li>
      ${libelle ? `<strong>${libelle}</strong>` : ''}
      ${liens || sousNiveaux ? `<ul>${liens}${sousNiveaux}</ul>` : ''}
    </li>`;
}

function renderArborescence(arbo: DossierLegislatifArborescence | undefined): string {
  if (!arbo) return '';
  const liensRacine = (arbo.liens ?? []).map(renderLien).join('');
  const niveaux = (arbo.niveaux ?? []).map(renderNiveau).join('');
  if (!liensRacine && !niveaux) return '';
  return `<section class="jur-section">
      <h2>Parcours parlementaire</h2>
      <ul class="jur-dossier-legislatif-tree">${liensRacine}${niveaux}</ul>
    </section>`;
}

function renderSousDossiers(dossiers: DossierLegislatifRef['dossiers']): string {
  const items = (dossiers ?? []).filter((d) => stripHtml(toHtmlString(d.contenuDossier)));
  if (!items.length) return '';
  return items.map((d) => {
    const libelle = stripHtml(toHtmlString(d.libelleTexte)) || 'Document';
    return `<section class="jur-section">
        <h2>${libelle}</h2>
        ${sanitizeHtml(linkifyLegifranceUrls(toHtmlString(d.contenuDossier)))}
      </section>`;
  }).join('');
}

/** Dossier législatif (JORFDOLE…) — réponse de POST /consult/dossierLegislatif,
 *  ou objet `DossierLegislatif` déjà embarqué sur un SearchResult/texte
 *  consulté (même forme dans les deux cas, cf. commentaire en tête de
 *  fichier). */
export function extractDossierLegislatifContent(obj: Record<string, unknown>, titre: string): ViewedContent {
  // Réponse de /consult/dossierLegislatif : enveloppée sous `dossierLegislatif`
  // (ConsultDossierLegislatifResponse). Si `obj` est déjà un DossierLegislatif
  // "nu" (cas d'un item de SearchResult.dossiersLegislatifs passé
  // directement), la clé est absente et on utilise `obj` tel quel.
  const dossier = ((obj.dossierLegislatif ?? obj) as unknown) as DossierLegislatifRef;

  const titreDossier = stripHtml(toHtmlString(dossier.titre)) || titre;
  const dateCreation = formatLegiDate(dossier.dateCreation);
  const dateMaj = formatLegiDate(dossier.dateDerniereModification);
  const nature = stripHtml(toHtmlString(dossier.nature));
  const typeDossier = stripHtml(toHtmlString(dossier.type));
  const legislature = stripHtml(toHtmlString(dossier.legislature?.libelle));
  const meta = metaHtml([
    ['Nature', nature || undefined],
    ['Statut', typeDossier || undefined],
    ['Législature', legislature || undefined],
    ['Création', dateCreation || undefined],
    ['Dernière modification', dateMaj || undefined],
  ]);

  const exposeHtml = stripHtml(toHtmlString(dossier.exposeMotif))
    ? `<section class="jur-section"><h2>Exposé des motifs</h2>${sanitizeHtml(linkifyLegifranceUrls(toHtmlString(dossier.exposeMotif)))}</section>`
    : '';

  const arborescenceHtml = renderArborescence(dossier.arborescence);
  const sousDossiersHtml = renderSousDossiers(dossier.dossiers);

  const hasAnyContent = !!(exposeHtml || arborescenceHtml || sousDossiersHtml);

  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${titreDossier}</h1>`,
      meta,
      exposeHtml,
      arborescenceHtml,
      sousDossiersHtml,
      !hasAnyContent
        ? `<p class="jur-muted">Aucun contenu détaillé (exposé des motifs, parcours parlementaire) n'est renseigné pour ce dossier législatif.</p>`
        : '',
    ].filter(Boolean).join('\n'),
  };
}
