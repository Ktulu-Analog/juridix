// ============================================================================
// Juridix — Espace "Recherche juridique" — Extraction Dossier législatif
// ============================================================================
// Fonds concerné : dossiers législatifs (id `JORFDOLE…`), travaux
// préparatoires d'une loi — exposé des motifs, arborescence des débats
// parlementaires par chambre (Sénat / Assemblée nationale), sous-dossiers
// avec leur propre contenu HTML.
//
// Point d'entrée : `SearchResult.dossiersLegislatifs` / `ViewedDoc.dossiersLegislatifs`
// (cf. roadmap A4), affiché via un bouton dédié dans l'en-tête de la
// visionneuse (JuridiquePanel.tsx) et sur les cartes de résultat LODA/JORF.
// Ouverture par id via GET /api/juridique/dossier/:id → POST
// /consult/dossierLegislatif côté PISTE (cf.
// legifrance.rs::get_dossier_legislatif, déjà câblé côté backend en
// routes.rs::get_dossier).
//
// Schéma vérifié le 08/07/2026 contre le swagger officiel Légifrance
// v2.4.2 (définitions `ConsultDossierLegislatifResponse`, `DossierLegislatif`,
// `Dossier`, `Arborescence`, `Niveau`, `Lien`, `Legislature`) — cf.
// useDocumentViewer.ts pour le repli entre `raw.dossiersLegislatifs` (présent
// directement sur ConsultTextResponse/ConsultJorfResponse) et
// `SearchResult.dossiersLegislatifs`. Les deux exposent des objets
// `DossierLegislatif` COMPLETS (pas de simples références id/titre) : la
// réponse de POST /consult/dossierLegislatif elle-même n'ajoute donc en
// pratique rien de plus que ce qui est déjà disponible depuis la recherche
// ou le texte consulté — l'appel réseau reste utile pour une navigation
// dédiée cohérente avec le reste de l'application (pile de documents,
// bouton retour) et pour couvrir les cas où le champ serait absent.
//
// Points à noter sur le schéma réel (par opposition à la première version de
// ce module, écrite avant d'avoir accès à cette doc officielle — cf. le
// commit associé si besoin de comparer) :
//  - La réponse de /consult/dossierLegislatif est enveloppée sous la clé
//    `dossierLegislatif` (`ConsultDossierLegislatifResponse.dossierLegislatif`),
//    pas directement à la racine.
//  - Pas de liste "actes législatifs" à plat : le parcours parlementaire est
//    une arborescence récursive (`arborescence.niveaux[].liens[]`,
//    `niveaux[].niveaux[]…`), typiquement un niveau par chambre (« Sénat »,
//    « Assemblée nationale ») contenant des liens externes (Sénat/AN/JO —
//    jamais des ids Légifrance résolubles en interne).
//  - `exposeMotif` (pas `exposeDesMotifs`) : HTML.
//  - `dossiers[]` : sous-dossiers avec leur propre contenu HTML
//    (`contenuDossier`), pas des documents PDF ; le champ `libelleTexte`
//    donne l'intitulé (ex. « Projet de loi »).
//  - Dates (`dateCreation`, `dateDerniereModification`, `dateMajEcheancier`)
//    en timestamp ms — déjà géré tel quel par `formatLegiDate`.
//
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
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
