// ============================================================================
// Juridix — Espace "Recherche juridique" — Types partagés
// ============================================================================
// Types purs (aucune logique), utilisés par JuridiquePanel.tsx et par les
// futurs hooks (useJuridiqueSearch, useDocumentViewer, useDossiers,
// useFavorites, useAnnotations…).
//
// Extrait de JuridiquePanel.tsx (étape 3 du refactor — cf. discussion du
// 04-05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import type { ReactNode } from 'react';
import type { ViewedContent } from './content/types';

// ── Dossier / cartable ───────────────────────────────────────────────────────
export interface DossierItem {
  id: string;
  titre: string;
  source: Source;
  type: string;
  contenu: string;
  addedAt: number;
  raw: unknown;
}

/** Une analyse LLM générée pour un dossier (synthèse, points clés,
 *  contradictoire…) — horodatée, conservée en historique plutôt qu'écrasée à
 *  chaque nouvelle demande, pour permettre de revenir sur une analyse
 *  antérieure même après avoir relancé une nouvelle synthèse ou modifié la
 *  composition du dossier entre-temps. */
export interface DossierAnalysis {
  id: string;
  mode: 'synthese' | 'contradictoire' | 'points_cles';
  message: string;           // réponse brute du LLM (markdown)
  createdAt: number;
  /** Titres des documents du dossier au moment de l'analyse (traçabilité :
   *  le contenu du dossier peut évoluer après coup — items ajoutés/retirés). */
  itemTitres: string[];
}

export interface Dossier {
  id: string;
  nom: string;
  createdAt: number;
  items: DossierItem[];
  /** Id de la conversation SQLite portant le fil contextuel de ce dossier. */
  conversationId?: string;
  /** Historique des analyses LLM de ce dossier, la plus récente en dernier. */
  analyses?: DossierAnalysis[];
}

// ── Annotations & surlignages ────────────────────────────────────────────────
export type AnnotationColor = 'yellow' | 'green' | 'blue' | 'pink';
export interface Annotation {
  id: string;
  docId: string;
  text: string;            // texte sélectionné
  note: string;            // commentaire optionnel
  color: AnnotationColor;
  createdAt: number;
  rangeStart: number;      // offset caractère dans contenu MD
  rangeEnd: number;
}

// ── Timeline ─────────────────────────────────────────────────────────────────
export interface TimelineVersion {
  id: string;
  dateDebut: string;       // timestamp ms (string) ou YYYY-MM-DD
  dateFin?: string;
  etat: string;
  isEndVersion?: boolean;
  modificateurs?: Array<{ id: string; title: string; action: string }>;
}

// A4 — schéma vérifié contre le swagger officiel Légifrance v2.4.2
// (définitions `DossierLegislatif` / `Lien` / `Niveau` / `Arborescence` /
// `Legislature` / `Dossier`, réponse `ConsultDossierLegislatifResponse` de
// POST /consult/dossierLegislatif). Le champ `dossiersLegislatifs` (déjà des
// objets `DossierLegislatif` COMPLETS, pas de simples références id/titre)
// est présent sur `SearchResult`, mais AUSSI directement sur les réponses de
// consultation d'un texte (`ConsultTextResponse`/`ConsultJorfResponse`) — cf.
// useDocumentViewer.ts::openDocument, qui lit `raw.dossiersLegislatifs` en
// priorité (plus à jour) et ne retombe sur celui du résultat de recherche
// que si absent.

/** Lien terminal dans l'arborescence d'un dossier législatif — pointe vers
 *  une ressource externe (Sénat/Assemblée nationale), pas vers un id
 *  Légifrance résoluble en interne. */
export interface DossierLegislatifLien {
  libelle?: string;
  id?: string;
  data?: string;
  lien?: string;
}

/** Nœud intermédiaire de l'arborescence (ex. « Sénat », « Assemblée
 *  nationale ») — récursif : peut contenir d'autres niveaux et/ou des liens. */
export interface DossierLegislatifNiveau {
  libelle?: string;
  id?: string;
  liens?: DossierLegislatifLien[];
  niveaux?: DossierLegislatifNiveau[];
}

export interface DossierLegislatifArborescence {
  liens?: DossierLegislatifLien[];
  niveaux?: DossierLegislatifNiveau[];
}

/** Sous-dossier avec son propre contenu HTML (ex. un dossier par chambre). */
export interface DossierLegislatifSousDossier {
  libelleTexte?: string;
  idTexte?: string;
  contenuDossier?: string;
}

/** Dossier législatif (id `JORFDOLE…`) — travaux préparatoires d'un texte de
 *  loi. Objet `DossierLegislatif` du swagger officiel. */
export interface DossierLegislatifRef {
  id: string;
  titre?: string;
  /** Texte HTML de l'exposé des motifs. */
  exposeMotif?: string;
  nature?: string;
  /** Ex. "LOI_PUBLIEE". */
  type?: string;
  /** Timestamps ms (nombre ou chaîne) — formatables tels quels via formatLegiDate. */
  dateCreation?: string;
  dateDerniereModification?: string;
  dateMajEcheancier?: string;
  echeancier?: string;
  origine?: string;
  url?: string;
  ancienId?: string;
  legislature?: { id?: string; libelle?: string; dateDebut?: string; dateFin?: string };
  dossiers?: DossierLegislatifSousDossier[];
  arborescence?: DossierLegislatifArborescence;
}

export interface SearchResult {
  id?: string;
  // Champ interne (non-API) : présent uniquement sur l'objet minimal passé
  // par handleCitationClick pour un lien de version d'article, afin de
  // router openDocument vers l'endpoint dédié /article-version/:cid/:id
  // plutôt que /consult/getArticle (qui ne renvoie pas fiablement le
  // contenu d'une version historique précise — cf. renderArticleVersionsBlock).
  _versionCid?: string;
  // ⚠️ titles[].id et titles[].cid identifient le TEXTE entier
  // (LEGITEXT/JORFTEXT), jamais un article. Cf. swagger SearchTitle.
  titles?: Array<{ id: string; title: string; cid?: string }>;
  sections?: Array<{
    id?: string;
    title?: string;
    extracts?: Array<{ id?: string; title?: string; num?: string; type?: string; values?: string[] }>;
  }>;
  title?: string;
  nature?: string;
  dateSignature?: string;
  datePublication?: string;
  date?: string;
  dateDecision?: string;
  // JUDILIBRE — nom de champ réel confirmé par le swagger officiel
  // (definitions decisionFull/searchResult) : il n'existe ni `date` ni
  // `dateDecision` côté Judilibre, uniquement `decision_date`/`decision_datetime`.
  decision_date?: string;
  decision_datetime?: string;
  summary?: string;
  titlesAndSummaries?: { titles?: unknown[]; secondaryTitles?: unknown[]; summary?: unknown };
  etat?: string;
  num?: string;
  nor?: string;
  text?: string;
  resumePrincipal?: string[];
  // LÉGIFRANCE — mots-clés associés au résultat (équivalent des `themes`
  // Judilibre pour les fonds où l'API les renseigne : LODA, JORF, CONVCOLL…
  // Vide pour la plupart des décisions de jurisprudence (JURI/CETAT/CONSTIT),
  // qui n'ont pas de taxonomie de matière côté Légifrance.
  motsCles?: string[];
  // JUDILIBRE
  score?: number;
  jurisdiction?: string;
  chamber?: string;
  formation?: string;
  solution?: string;
  ecli?: string;
  themes?: string[];
  publication?: string[];
  numbers?: string[];
  titrage?: Array<{ titre?: string; theme?: string; originalText?: string }> | string[];
  highlights?: Record<string, string[]>;
  // A4 — présent sur les résultats LODA/JORF (lois/décrets), absent des
  // décisions de jurisprudence : pointe vers le(s) dossier(s) législatif(s)
  // (travaux préparatoires — exposé des motifs, arborescence des débats
  // parlementaires par chambre) liés à ce texte. Déjà des objets complets
  // (pas de simples id/titre) — cf. DossierLegislatifRef ci-dessus, vérifié
  // contre le swagger officiel Légifrance v2.4.2.
  dossiersLegislatifs?: DossierLegislatifRef[];
}

export interface SearchResponse {
  total?: number;
  totalResultNumber?: number;
  results?: SearchResult[];
  error?: string;
  // JUDILIBRE
  next_batch?: number;
  previous_batch?: number;
}

export type Source = 'legifrance' | 'judilibre';
export type Fond =
  | 'LODA_ETAT' | 'LODA_DATE'
  | 'CODE_ETAT' | 'CODE_DATE'
  | 'JORF'
  | 'JURI' | 'CETAT' | 'CONSTIT' | 'JUFI'
  | 'CNIL' | 'KALI' | 'CIRC' | 'ACCO'
  | 'BOCC' | 'DEBAT' | 'QUESTION_ECRITE' | 'DOC_ADMIN'
  | 'ALL';

// ── Table des matières flottante ─────────────────────────────────────────────
export interface TocEntry {
  id: string;
  label: string;
  level: number; // 0 = titre du document, 1 = section/zone
}

// ── Plan interactif (sommaire façon Dalloz) ──────────────────────────────────
// Arbre reconstruit depuis le DOM rendu (cf. buildTocTree dans
// useDocumentViewer.ts) en respectant l'imbrication réelle des sections —
// contrairement à `TocEntry` (liste plate historique), qui aplatissait tous
// les niveaux de profondeur d'un code/texte en un seul "level 1" indistinct.
export interface TocTreeNode {
  id: string;
  label: string;
  level: number;                 // profondeur réelle dans l'arbre (0 = racine)
  type: 'section' | 'article';
  children: TocTreeNode[];
}

export interface ViewedDoc {
  id: string;
  titre: string;
  content: ViewedContent;   // HTML sanitisé ou Markdown pur
  contenu: string;          // texte brut pour LLM (markdown extrait)
  source: Source;
  type: 'article' | 'decision' | 'code' | 'loda' | 'juri' | 'kali' | 'cnil' | 'circ' | 'acco' | 'bocc' | 'debat' | 'question_ecrite' | 'doc_admin' | 'dossier_legislatif';
  raw: unknown;
  // A4 — le champ `dossiersLegislatifs` est présent à la fois sur le
  // SearchResult ET directement sur la réponse de consultation du texte
  // (ConsultTextResponse/ConsultJorfResponse, cf. swagger officiel) : reporté
  // ici au moment de l'ouverture avec priorité à `raw.dossiersLegislatifs`
  // (plus à jour), repli sur celui du résultat de recherche sinon — cf.
  // useDocumentViewer.ts::openDocument. Permet d'afficher le bouton
  // « Travaux préparatoires » une fois le texte affiché en plein écran.
  dossiersLegislatifs?: DossierLegislatifRef[];
  // ── Résolution inline des citations d'article (cf. resolveInlineCitations
  // dans content/html-helpers.ts) — champs permettant de proposer un bouton
  // « Charger tous les articles » qui relance la résolution sans plafond
  // quand un document cite plus d'articles que le plafond par défaut :
  //  - rawBody : HTML AVANT résolution inline (citations encore de simples
  //    liens), conservé uniquement pour pouvoir relancer la résolution ;
  //    absent si le document ne contient aucune citation d'article.
  //  - totalInlineArticles : nombre total (non plafonné) d'articles cités.
  //  - allArticlesLoaded : true une fois la résolution complète effectuée
  //    (soit parce que le total ne dépassait pas le plafond, soit après un
  //    clic sur le bouton « Charger tous les articles »).
  rawBody?: string;
  totalInlineArticles?: number;
  allArticlesLoaded?: boolean;
}

/** Type de document affiché dans la visionneuse (alias de ViewedDoc['type']). */
export type DocType = ViewedDoc['type'];

export interface JuridiquePanelProps {
  /** Rappelé pour envoyer un message (document, synthèse de dossier…) dans le chat
   *  principal partagé — la réponse y apparaît avec artefacts, historique, RAG, MCP.
   *  Retourne (une fois le streaming terminé) le texte final généré par le LLM,
   *  pour permettre à l'appelant (ex : historique des analyses de dossier) de
   *  conserver la VRAIE réponse plutôt que le prompt envoyé. `null` si l'appel
   *  a été interrompu/en erreur. */
  onSendToChat: (message: string) => Promise<string | null>;
  /** Rappelé pour RÉAFFICHER un message déjà généré (ex : relecture d'une
   *  analyse de dossier archivée) sans relancer d'appel LLM — contrairement à
   *  `onSendToChat`, le texte est injecté tel quel comme réponse d'assistant,
   *  pas envoyé en tant que nouvelle question. */
  onShowInChat: (message: string) => void;
  onClose: () => void;
  /** Contrôles transverses (usage tokens/CO₂, artefacts, comparer, utilisateur, modèle)
   *  fusionnés dans l'en-tête du panneau pour éviter d'empiler deux bandeaux. */
  globalControls?: ReactNode;
}
