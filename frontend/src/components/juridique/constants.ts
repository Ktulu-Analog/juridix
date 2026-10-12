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
// Espace "Recherche juridique" — Constantes
// ============================================================================
// Options de fonds, tris, filtres, taxonomies statiques et libellés.
// ============================================================================

import type { Fond, ViewedDoc } from './types';

export const FONDS_LEGIFRANCE: { value: Fond; label: string }[] = [
  { value: 'LODA_ETAT', label: 'Lois & décrets (état)' },
  { value: 'LODA_DATE', label: 'Lois & décrets (date)' },
  { value: 'CODE_ETAT', label: 'Codes (état juridique)' },
  { value: 'CODE_DATE', label: 'Codes (par date)' },
  { value: 'JORF', label: 'Journal officiel (JORF)' },
  { value: 'JURI', label: 'Jurisprudence judiciaire' },
  { value: 'CETAT', label: 'Jurisprudence admin. (CE)' },
  { value: 'CONSTIT', label: 'Jurisprudence constit.' },
  { value: 'JUFI', label: 'Jurisprudence financière' },
  { value: 'CNIL', label: 'CNIL' },
  { value: 'KALI', label: 'Conv. collectives (KALI)' },
  { value: 'CIRC', label: 'Circulaires' },
  { value: 'ACCO', label: 'Accords d\'entreprise' },
  { value: 'BOCC', label: 'Bull. Off. Conv. Coll. (BOCC)' },
  { value: 'DEBAT', label: 'Débats parlementaires' },
  { value: 'QUESTION_ECRITE', label: 'Questions écrites parl.' },
  { value: 'DOC_ADMIN', label: 'Documents administratifs' },
  { value: 'ALL', label: 'Tous les fonds' },
];

export const SORTS_LEGIFRANCE = [
  { value: 'PERTINENCE',            label: 'Pertinence' },
  { value: 'SIGNATURE_DATE_DESC',   label: 'Date signature ↓' },
  { value: 'SIGNATURE_DATE_ASC',    label: 'Date signature ↑' },
  { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
  { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
  { value: 'DATE_DESC',             label: 'Date décision ↓' },
  { value: 'DATE_ASC',              label: 'Date décision ↑' },
  { value: 'MODIFICATION_DATE_DESC', label: 'Modification ↓' },
  { value: 'PUBLI_DATE_DESC',       label: 'Mise en ligne ↓' },
  { value: 'PUBLI_DATE_ASC',        label: 'Mise en ligne ↑' },
  { value: 'DATE_DECISION_DESC',    label: 'Décision ↓ (CNIL)' },
  { value: 'DATE_DECISION_ASC',     label: 'Décision ↑ (CNIL)' },
  { value: 'DEBAT_PARLEMENTAIRE_DESC', label: 'Débat ↓' },
  { value: 'DEBAT_PARLEMENTAIRE_ASC',  label: 'Débat ↑' },
  { value: 'QUESTION_ECRITE_PARLEMENTAIRE_DESC', label: 'Question ↓' },
  { value: 'QUESTION_ECRITE_PARLEMENTAIRE_ASC',  label: 'Question ↑' },
  { value: 'BOCC_SORT_DESC',        label: 'BOCC ↓' },
  { value: 'BOCC_SORT_ASC',         label: 'BOCC ↑' },
  { value: 'ID_DESC',               label: 'ID ↓' },
  { value: 'ID_ASC',                label: 'ID ↑' },
];

export const SORTS_JUDILIBRE = [
  { value: 'scorepub', label: 'Pertinence + Publication' },
  { value: 'score',    label: 'Pertinence' },
  { value: 'date',     label: 'Date' },
];

// Tris valides PAR FONDS, d'après le classeur officiel « description des
// tris et filtres de l'API » (une feuille par fonds, section « Type de
// tri »). Certains fonds n'ont AUCUNE section de tri dans le classeur —
// ALL, CODE_DATE/CODE_ETAT, DOC_ADMIN — ce qui signifie que l'API ne permet
// aucun tri explicite pour eux (seul l'ordre de pertinence implicite existe).
// Utiliser un tri hors de cette liste pour un fonds donné n'est pas
// garanti par l'API (au mieux ignoré, au pire une erreur), d'où le filtrage
// du sélecteur ci-dessous au lieu d'une liste unique valable pour tous les
// fonds.
export const SORTS_BY_FOND: Partial<Record<Fond, { value: string; label: string }[]>> = {
  JORF: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'SIGNATURE_DATE_DESC',   label: 'Date signature ↓' },
    { value: 'SIGNATURE_DATE_ASC',    label: 'Date signature ↑' },
    { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
    { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
  ],
  LODA_DATE: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
    { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
    { value: 'SIGNATURE_DATE_DESC',   label: 'Date signature ↓' },
    { value: 'SIGNATURE_DATE_ASC',    label: 'Date signature ↑' },
  ],
  LODA_ETAT: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
    { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
    { value: 'SIGNATURE_DATE_DESC',   label: 'Date signature ↓' },
    { value: 'SIGNATURE_DATE_ASC',    label: 'Date signature ↑' },
  ],
  CIRC: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'SIGNATURE_DATE_DESC', label: 'Date signature ↓' },
    { value: 'SIGNATURE_DATE_ASC',  label: 'Date signature ↑' },
    { value: 'PUBLI_DATE_DESC',     label: 'Mise en ligne ↓' },
    { value: 'PUBLI_DATE_ASC',      label: 'Mise en ligne ↑' },
  ],
  CONSTIT: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DESC', label: 'Date décision ↓' },
    { value: 'DATE_ASC',  label: 'Date décision ↑' },
  ],
  CETAT: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DESC', label: 'Date décision ↓' },
    { value: 'DATE_ASC',  label: 'Date décision ↑' },
  ],
  JURI: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DESC', label: 'Date décision ↓' },
    { value: 'DATE_ASC',  label: 'Date décision ↑' },
  ],
  JUFI: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DESC', label: 'Date décision ↓' },
    { value: 'DATE_ASC',  label: 'Date décision ↑' },
  ],
  KALI: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'MODIFICATION_DATE_DESC', label: 'Modification ↓' },
    { value: 'SIGNATURE_DATE_DESC',    label: 'Date signature ↓' },
    { value: 'SIGNATURE_DATE_ASC',     label: 'Date signature ↑' },
  ],
  ACCO: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DESC', label: 'Date signature ↓' },
    { value: 'DATE_ASC',  label: 'Date signature ↑' },
  ],
  CNIL: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'DATE_DECISION_DESC', label: 'Décision ↓ (CNIL)' },
    { value: 'DATE_DECISION_ASC',  label: 'Décision ↑ (CNIL)' },
  ],
  BOCC: [
    { value: 'BOCC_SORT_DESC', label: 'BOCC ↓' },
    { value: 'BOCC_SORT_ASC',  label: 'BOCC ↑' },
  ],
  DEBAT: [
    { value: 'DEBAT_PARLEMENTAIRE_DESC', label: 'Débat ↓' },
    { value: 'DEBAT_PARLEMENTAIRE_ASC',  label: 'Débat ↑' },
    { value: 'ID_DESC', label: 'ID ↓' },
    { value: 'ID_ASC',  label: 'ID ↑' },
  ],
  QUESTION_ECRITE: [
    { value: 'QUESTION_ECRITE_PARLEMENTAIRE_DESC', label: 'Question ↓' },
    { value: 'QUESTION_ECRITE_PARLEMENTAIRE_ASC',  label: 'Question ↑' },
    { value: 'ID_DESC', label: 'ID ↓' },
    { value: 'ID_ASC',  label: 'ID ↑' },
  ],
  // NB : le classeur officiel ne contient, à la lettre, aucune section
  // « Type de tri » pour la feuille CODE (contrairement à LODA/JORF...).
  // Le backend (SearchFond::valid_sorts) autorise cependant déjà
  // PERTINENCE/PUBLICATION_DATE_DESC/PUBLICATION_DATE_ASC pour ce fond — on
  // aligne le sélecteur sur ce comportement existant plutôt que de le
  // contredire, mais ce point mériterait une vérification live contre l'API
  // (voir le message d'accompagnement).
  CODE_DATE: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
    { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
  ],
  CODE_ETAT: [
    { value: 'PERTINENCE', label: 'Pertinence' },
    { value: 'PUBLICATION_DATE_DESC', label: 'Publication ↓' },
    { value: 'PUBLICATION_DATE_ASC',  label: 'Publication ↑' },
  ],
  // ALL, DOC_ADMIN : volontairement absents — le classeur officiel ne
  // documente aucune section « Type de tri » pour ces 2 fonds (seule la
  // pertinence implicite existe), et rien côté backend ne contredit ce
  // constat pour eux (contrairement à CODE ci-dessus).
};

export const JURISDICTIONS_JUDILIBRE = [
  { value: 'cc', label: 'Cour de cassation' },
  { value: 'ca', label: 'Cours d\'appel' },
  { value: 'tj', label: 'Tribunaux judiciaires' },
  { value: 'ce', label: 'Conseil d\'État' },
];

export const PUBLICATIONS_JUDILIBRE = [
  { value: 'b',  label: 'Bulletin' },
  { value: 'r',  label: 'Rapport' },
  { value: 'l',  label: 'Lettre' },
  { value: 'c',  label: 'Communiqué' },
];

export const SOLUTIONS_JUDILIBRE = [
  { value: 'cassation',       label: 'Cassation' },
  { value: 'rejet',           label: 'Rejet' },
  { value: 'annulation',      label: 'Annulation' },
  { value: 'irrecevabilite',  label: 'Irrecevabilité' },
  { value: 'qpc',             label: 'QPC' },
  { value: 'avis',            label: 'Avis' },
  { value: 'decheance',       label: 'Déchéance' },
  { value: 'designation',     label: 'Désignation' },
  { value: 'nonlieu',         label: 'Non-lieu' },
  { value: 'rabat',           label: 'Rabat' },
];

// Nature de texte — Légifrance. Enum officiel (swagger LODAListRequest.natures).
// D'après la doc officielle, seuls les fonds JORF, LODA_DATE et LODA_ETAT
// exposent ce filtre (cf. SearchFond::nature_facette côté backend) ; JORF
// partage la même nomenclature de nature de texte que LODA.
export const NATURES_LEGIFRANCE = [
  { value: 'LOI',                      label: 'Loi' },
  { value: 'ORDONNANCE',               label: 'Ordonnance' },
  { value: 'DECRET',                   label: 'Décret' },
  { value: 'DECRET_LOI',               label: 'Décret-loi' },
  { value: 'ARRETE',                   label: 'Arrêté' },
  { value: 'CONSTITUTION',             label: 'Constitution' },
  { value: 'DECISION',                 label: 'Décision' },
  { value: 'CONVENTION',               label: 'Convention' },
  { value: 'DECLARATION',              label: 'Déclaration' },
  { value: 'ACCORD_FONCTION_PUBLIQUE', label: 'Accord fonction publique' },
];

// État juridique — Légifrance. Enum officiel (legalStatus), commun à
// LODA_ETAT/CODE_ETAT (facette TEXT_LEGAL_STATUS) et KALI (facette
// LEGAL_STATUS). Cf. SearchFond::legal_status_facette côté backend.
export const LEGAL_STATUS_LEGIFRANCE = [
  { value: 'VIGUEUR',          label: 'En vigueur' },
  { value: 'VIGUEUR_DIFF',     label: 'Vigueur différée' },
  { value: 'VIGUEUR_ETEN',     label: 'Vigueur étendue' },
  { value: 'VIGUEUR_NON_ETEN', label: 'Vigueur non étendue' },
  { value: 'ABROGE',           label: 'Abrogé' },
  { value: 'ABROGE_DIFF',      label: 'Abrogation différée' },
  { value: 'PERIME',           label: 'Périmé' },
  { value: 'ANNULE',           label: 'Annulé' },
  { value: 'MODIFIE',          label: 'Modifié' },
  { value: 'MODIFIE_MORT_NE',  label: 'Modifié (mort-né)' },
  { value: 'DISJOINT',         label: 'Disjoint' },
  { value: 'SUBSTITUE',        label: 'Substitué' },
  { value: 'TRANSFERE',        label: 'Transféré' },
  { value: 'DENONCE',          label: 'Dénoncé' },
  { value: 'REMPLACE',         label: 'Remplacé' },
  { value: 'INITIALE',         label: 'Version initiale' },
  { value: 'SANS_ETAT',        label: 'Sans état' },
];

// Zones de contenu ciblées par la recherche JUDILIBRE (paramètre `field`).
// Valeurs officielles d'après la description du endpoint /search et le
// schéma `zone` du swagger (introduction, expose, moyens, motivations,
// dispositif, annexes) + sommaire/titrage mentionnés dans la description.
export const FIELDS_JUDILIBRE = [
  { value: 'titrage',      label: 'Titrage' },
  { value: 'sommaire',     label: 'Sommaire' },
  { value: 'expose',       label: 'Exposé du litige' },
  { value: 'moyens',       label: 'Moyens' },
  { value: 'motivations',  label: 'Motivations' },
  { value: 'dispositif',   label: 'Dispositif' },
  { value: 'annexes',      label: 'Moyens annexés' },
];

// Opérateur logique entre les termes de la recherche JUDILIBRE.
export const OPERATEURS_JUDILIBRE = [
  { value: 'or',    label: 'Au moins un des termes (OU)' },
  { value: 'and',   label: 'Tous les termes (ET)' },
  { value: 'exact', label: 'Expression exacte' },
];

// Types de documents associés aux décisions JUDILIBRE (paramètre
// `withFileOfType`). Valeurs et libellés officiels d'après la description
// du endpoint /search.
export const FILE_TYPES_JUDILIBRE = [
  { value: 'comm_comm', label: 'Communiqué' },
  { value: 'comm_note', label: 'Note explicative' },
  { value: 'comm_nora', label: 'Notice au rapport annuel' },
  { value: 'comm_lett', label: 'Lettre de chambre' },
  { value: 'comm_trad', label: 'Arrêt traduit' },
  { value: 'prep_rapp', label: 'Rapport du rapporteur' },
  { value: 'prep_avis', label: 'Avis de l\u2019avocat général' },
  { value: 'prep_oral', label: 'Avis oral de l\u2019avocat général' },
];

// Types de publication parlementaire (débats / questions écrites). Valeurs
// standards DILA : AN (Assemblée nationale) / SENAT (Sénat).
export const TYPES_PARLEMENT = [
  { value: 'AN',    label: 'Assemblée nationale' },
  { value: 'SENAT', label: 'Sénat' },
];

// Nature de décision JUDILIBRE — repli statique minimal si le chargement de
// la taxonomie (GET /taxonomie/type) échoue. Liste non exhaustive : la
// source de vérité reste la taxonomie dynamique.
export const DECISION_TYPES_FALLBACK = [
  { value: 'arret',      label: 'Arrêt' },
  { value: 'ordonnance', label: 'Ordonnance' },
  { value: 'qpc',        label: 'QPC' },
  { value: 'saisie',     label: 'Saisine pour avis' },
  { value: 'avis',       label: 'Avis' },
];

// Fonds Légifrance ne supportant AUCUNE recherche plein texte côté API
// (uniquement des filtres structurels : dates, IDCC, type de parlement,
// années). Cf. swagger officiel : ces fonds n'apparaissent pas dans l'enum
// `SearchRequestDTO.fond` et leurs requêtes de liste (`DebatsParlementairesListRequest`,
// `QuestionsEcritesParlementairesListRequest`, `DocsAdminsListRequest`) n'ont
// aucun champ de texte libre.
export const FONDS_SANS_RECHERCHE_TEXTUELLE: Fond[] = ['DEBAT', 'QUESTION_ECRITE', 'DOC_ADMIN'];

export const FONDS_AVEC_NATURE: Fond[] = ['JORF', 'LODA_DATE', 'LODA_ETAT'];
export const FONDS_AVEC_ETAT_JURIDIQUE: Fond[] = ['LODA_ETAT', 'CODE_ETAT', 'KALI'];

// Champs de recherche disponibles par fond (depuis le fichier Excel)
export const CHAMPS_PAR_FOND: Record<string, { value: string; label: string }[]> = {
  ALL:            [{ value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' }],
  JORF:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NOR', label: 'NOR' }, { value: 'NUM', label: 'N° de texte' },
    { value: 'NUM_ARTICLE', label: 'N° d\'article' }, { value: 'ARTICLE', label: 'Contenu article' },
    { value: 'VISA', label: 'Visas' }, { value: 'NOTICE', label: 'Notices' },
    { value: 'VISA_NOTICE', label: 'Visas ou notices' }, { value: 'TRAVAUX_PREP', label: 'Travaux préparatoires' },
    { value: 'SIGNATURE', label: 'Signatures' }, { value: 'NOTA', label: 'Notas' },
  ],
  CODE_DATE:      [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'TABLE', label: 'Table des matières' }, { value: 'NUM_ARTICLE', label: 'N° d\'article' },
    { value: 'ARTICLE', label: 'Contenu article' },
  ],
  CODE_ETAT:      [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'TABLE', label: 'Table des matières' }, { value: 'NUM_ARTICLE', label: 'N° d\'article' },
    { value: 'ARTICLE', label: 'Contenu article' },
  ],
  LODA_DATE:      [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NOR', label: 'NOR' }, { value: 'NUM', label: 'N° de texte' },
    { value: 'NUM_ARTICLE', label: 'N° d\'article' }, { value: 'ARTICLE', label: 'Contenu article' },
    { value: 'VISA', label: 'Visas' }, { value: 'NOTICE', label: 'Notices' },
    { value: 'VISA_NOTICE', label: 'Visas ou notices' }, { value: 'TRAVAUX_PREP', label: 'Travaux préparatoires' },
    { value: 'SIGNATURE', label: 'Signatures' }, { value: 'NOTA', label: 'Notas' },
  ],
  LODA_ETAT:      [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NOR', label: 'NOR' }, { value: 'NUM', label: 'N° de texte' },
    { value: 'NUM_ARTICLE', label: 'N° d\'article' }, { value: 'ARTICLE', label: 'Contenu article' },
    { value: 'VISA', label: 'Visas' }, { value: 'NOTICE', label: 'Notices' },
    { value: 'VISA_NOTICE', label: 'Visas ou notices' }, { value: 'TRAVAUX_PREP', label: 'Travaux préparatoires' },
    { value: 'SIGNATURE', label: 'Signatures' }, { value: 'NOTA', label: 'Notas' },
  ],
  CIRC:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NOR', label: 'NOR' }, { value: 'RESUME_CIRC', label: 'Résumé' },
    { value: 'TEXTE_REF', label: 'Textes de référence' },
  ],
  CONSTIT:        [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NUM_DEC', label: 'N° de décision' }, { value: 'TEXTE', label: 'Contenu' },
  ],
  CETAT:          [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NUM_DEC', label: 'N° de décision' }, { value: 'ABSTRATS', label: 'Abstracts' },
    { value: 'TEXTE', label: 'Contenu' }, { value: 'RESUMES', label: 'Résumés' },
  ],
  JURI:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'ABSTRATS', label: 'Abstracts' }, { value: 'NUM_AFFAIRE', label: 'N° d\'affaire' },
    { value: 'TEXTE', label: 'Contenu' }, { value: 'RESUMES', label: 'Résumés' },
  ],
  JUFI:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NUM_DEC', label: 'N° de décision' }, { value: 'ABSTRATS', label: 'Abstracts' },
    { value: 'TEXTE', label: 'Contenu' },
  ],
  KALI:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'IDCC', label: 'IDCC' }, { value: 'MOTS_CLES', label: 'Mots clés' },
    { value: 'ARTICLE', label: 'Contenu article' },
  ],
  ACCO:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'RAISON_SOCIALE', label: 'Raison sociale' }, { value: 'IDCC', label: 'IDCC' },
  ],
  CNIL:           [
    { value: 'ALL', label: 'Tous les champs' }, { value: 'TITLE', label: 'Titre' },
    { value: 'NOR', label: 'NOR' }, { value: 'TEXTE', label: 'Contenu' },
    { value: 'NUM_DELIB', label: 'N° de délibération' },
  ],
};

export const FOND_LABELS: Record<Fond, string> = {
  LODA_ETAT: 'Lois & décrets', LODA_DATE: 'Lois & décrets',
  CODE_ETAT: 'Codes', CODE_DATE: 'Codes',
  JORF: 'Journal officiel', JURI: 'Jurisprudence judiciaire',
  CETAT: 'Jurisprudence administrative', CONSTIT: 'Jurisprudence constitutionnelle',
  JUFI: 'Jurisprudence financière', CNIL: 'CNIL', KALI: 'Conventions collectives',
  CIRC: 'Circulaires', ACCO: "Accords d'entreprise",
  BOCC: 'Bull. Off. Conv. Coll.', DEBAT: 'Débats parlementaires',
  QUESTION_ECRITE: 'Questions écrites parl.', DOC_ADMIN: 'Documents administratifs',
  ALL: 'Tous les fonds',
};

export const DOC_TYPE_LABELS: Record<ViewedDoc['type'], string> = {
  article: 'Article', decision: 'Décision', code: 'Code', loda: 'Texte',
  juri: 'Décision', kali: 'Convention collective', cnil: 'Délibération CNIL',
  circ: 'Circulaire', acco: "Accord d'entreprise",
  bocc: 'Bulletin BOCC', debat: 'Débat parlementaire',
  question_ecrite: 'Question écrite', doc_admin: 'Document administratif',
  // A4 — dossier législatif (travaux préparatoires d'une loi), ouvert depuis
  // SearchResult.dossiersLegislatifs (cf. roadmap A4 et content/extract-
  // dossier-legislatif.ts pour les réserves sur le schéma exact).
  dossier_legislatif: 'Dossier législatif',
};

/** Libellés abrégés des indicateurs de publication Judilibre. */
export const PUB_LABELS: Record<string, string> = { b: 'Bull.', r: 'Rapp.', l: 'Lettre', c: 'Comm.' };
