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
// Juridix — Espace "Recherche juridique" — Types partagés
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
  conversationId?: string;
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

export interface DossierLegislatifLien {
  libelle?: string;
  id?: string;
  data?: string;
  lien?: string;
}


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

export interface DossierLegislatifSousDossier {
  libelleTexte?: string;
  idTexte?: string;
  contenuDossier?: string;
}

export interface DossierLegislatifRef {
  id: string;
  titre?: string;
  exposeMotif?: string;
  nature?: string;
  type?: string;
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
  _versionCid?: string;
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
  decision_date?: string;
  decision_datetime?: string;
  summary?: string;
  titlesAndSummaries?: { titles?: unknown[]; secondaryTitles?: unknown[]; summary?: unknown };
  etat?: string;
  num?: string;
  nor?: string;
  text?: string;
  resumePrincipal?: string[];
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
  dossiersLegislatifs?: DossierLegislatifRef[];
}

/** Méta-informations d'une recherche dans les codes (article par numéro / plein texte). */
export interface RechercheCodeMeta {
  mode: 'article' | 'texte';
  code?: string | null;
  /** Palier plein texte retenu ; à renvoyer (`recherche_palier`) pour les pages suivantes. */
  palier?: 'exacte' | 'tous_les_mots' | 'un_des_mots' | null;
  note?: string | null;
}

export interface SearchResponse {
  recherche?: RechercheCodeMeta;
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
  dossiersLegislatifs?: DossierLegislatifRef[];
  rawBody?: string;
  totalInlineArticles?: number;
  allArticlesLoaded?: boolean;
}

/** Type de document affiché dans la visionneuse (alias de ViewedDoc['type']). */
export type DocType = ViewedDoc['type'];

export interface JuridiquePanelProps {

  onSendToChat: (message: string, opts?: { completeWithMcp?: boolean }) => Promise<string | null>;
  officialMcpAvailable?: boolean;
  veilleSignal?: number;
  veilleUnread?: number;
  onVeilleChanged?: () => void;
  onShowInChat: (message: string) => void;
  onClose: () => void;
  globalControls?: ReactNode;
  winnowEnabled: boolean;
}

export interface WinnowDroppedItem {
  doc: number;
  doc_titre: string;
  heading: string | null;
  reason: string;       // « hors sujet » | « sans contenu juridique »
  p: number;            // probabilité sur laquelle repose l'éviction du passage
  preview: string;      // 120 premiers caractères du passage écarté
}

export interface WinnowReport {
  enabled: boolean;
  applied: boolean;
  model: string;
  total_passages: number;
  scored: number;
  kept: number;
  dropped: number;
  unscored: number;
  chars_before: number;
  chars_after: number;
  elapsed_ms: number;
  fallback: string | null;
  dropped_items: WinnowDroppedItem[];
}

export interface WinnowStatus {
  enabled: boolean;
  reachable: boolean;
  version: string | null;
  model: string;
  model_installed: boolean;
  model_loaded: boolean;
  device: string | null;
  hint: string | null;
}
