// ============================================================================
// Juridix — Vérification des références d'articles citées par le LLM
// ============================================================================
// Côté frontend du chantier 4 : appelle POST /api/juridique/verifier-citations
// en tâche de fond, puis annote les références dans le Markdown affiché.
//
// Principe : la vérification n'est jamais bloquante ni bruyante. Toute
// indisponibilité (PISTE non configuré, hors ligne, erreur) ⇒ aucun badge, le
// message reste affiché tel quel.
// ============================================================================

import { useEffect, useState } from 'react';
import { API_BASE } from '../constants';

export type CitationStatut =
  | 'en_vigueur' | 'abroge' | 'introuvable' | 'non_verifiable' | 'erreur'
  // Décisions, conventions collectives et citations textuelles :
  | 'verifiee'      // existe / passage retrouvé tel quel
  | 'incoherente'   // la source existe mais ce qui est cité ne concorde pas
  | 'approchante'   // citation proche de la source sans lui être identique
  | 'non_retrouvee';// recherche aboutie sans correspondance (≠ inexistant : à vérifier)

export interface CitationOccurrence {
  /** Offsets en unités UTF-16 (= String.prototype.slice). */
  debut: number;
  fin: number;
  texte: string;
}

/**
 * article    = article de code ;
 * texte      = loi / ordonnance / décret (existence vérifiée si cité par son numéro), autres textes (ancrage seul) ;
 * decision   = décision de la Cour de cassation citée par son numéro de pourvoi (Judilibre) ;
 * convention = convention collective citée par son IDCC ;
 * citation   = passage entre guillemets comparé à l'article ou aux documents fournis.
 */
export type CitationKind = 'article' | 'texte' | 'decision' | 'convention' | 'citation';

/** Présence dans ce qui a été fourni au modèle (documents, outils MCP, extraits RAG). */
export type CitationAncrage = 'dans_sources' | 'hors_sources';

export interface CitationVerification {
  kind: CitationKind;
  /** `null` : ancrage non évalué (conversation sans documents fournis). */
  ancrage: CitationAncrage | null;
  reference: string;
  code: string | null;
  numero: string;
  statut: CitationStatut;
  existe: boolean | null;
  etat: string | null;
  etat_libelle: string | null;
  date_fin: string | null;
  url: string | null;
  existe_dans: string[];
  detail: string | null;
  /** Identité de la source retrouvée (titre, chambre, date, solution…). */
  infos?: string[];
  occurrences: CitationOccurrence[];
}

export interface CitationReport {
  date: string;
  verifications: CitationVerification[];
}

/**
 * Ce que le modèle a reçu pour répondre. `null` = ne pas évaluer l'ancrage.
 * - `sources` : textes des messages utilisateur (prompt de dossier, pièces jointes…) ;
 * - `keys`    : clés déjà calculées par le backend de chat (résultats d'outils MCP,
 *               extraits RAG) — voir `grounding_keys` côté Rust.
 */
export interface CitationGrounding {
  sources: string[];
  keys: string[];
}

/** Évalué paresseusement, au moment de lancer la vérification (pas à chaque rendu). */
export type GroundingGetter = () => CitationGrounding | null;
/** Question à laquelle répond un message (conversation libre, sans documents fournis). */
export type QuestionGetter = () => string | null;

/** Préfixe des ancres produites par `annotateCitations` (lues par le rendu). */
export const CITE_HREF_PREFIX = '#juridix-cite-';

// Pré-filtre bon marché : évite un aller-retour réseau pour les messages sans
// aucune référence plausible (le backend refait l'extraction complète).
const QUICK_RE = new RegExp(
  [
    String.raw`\b(?:articles?|arts?)\b\.?\s*\d`,                 // article 1103
    String.raw`\b[LRDA]\s?\.?\s?\d{2,}`,                         // L. 1234-5
    String.raw`\bpourvoi\b|\bcass(?:ation)?\b|\bIDCC\b`,        // décisions, conventions collectives
    String.raw`[«“][^»”]{25,}[»”]|"[^"\n]{25,}"`,                // citations textuelles (« », “ ”, " ")
    // loi / décret / arrêté… suivi (de près) d'un numéro ou d'une date
    String.raw`\b(?:loi|ordonnance|d[ée]cret|arr[êe]t[ée]|circulaire|instruction|d[ée]cision)(?![a-zà-ÿ])[^\n]{0,40}\d`,  // \b ne voit pas le é final (ASCII seulement)
  ].join('|'),
  'i',
);

export function mightContainCitation(text: string): boolean {
  return QUICK_RE.test(text);
}

/** Marque d'existence (articles de codes). Vide si aucun verdict d'existence. */
export function citationBadge(v: CitationVerification): { icon: string; label: string } {
  switch (v.statut) {
    case 'en_vigueur':   return { icon: '✅', label: 'en vigueur' };
    case 'abroge':       return { icon: '⚠️', label: v.etat_libelle || 'plus en vigueur' };
    case 'introuvable':  return { icon: '❌', label: 'introuvable' };
    case 'verifiee':     return { icon: '✅', label: v.kind === 'citation' ? 'passage retrouvé' : 'vérifiée' };
    case 'incoherente':  return { icon: '⚠️', label: 'incohérente' };
    case 'approchante':  return { icon: '⚠️', label: 'passage proche' };
    case 'non_retrouvee':return { icon: '❓', label: 'non retrouvée' };
    default:             return { icon: '', label: '' };
  }
}

/** Marque d'ancrage : uniquement pour ce qui est absent des sources fournies. */
export function ancrageBadge(v: CitationVerification): { icon: string; label: string } | null {
  return v.ancrage === 'hors_sources' ? { icon: '🔎', label: 'hors sources' } : null;
}

const ANCRAGE_HORS =
  "Absent des sources fournies au modèle (documents, résultats d'outils, extraits) : cette référence vient de sa mémoire — à vérifier.";
const ANCRAGE_DANS = 'Présent dans les sources fournies au modèle.';

/** Texte d'info-bulle complet. */
export function citationTitle(v: CitationVerification): string {
  const ref = v.reference;
  let main: string;
  switch (v.statut) {
    case 'en_vigueur':
      main = `${ref} — existe et est en vigueur (Légifrance). Cliquer pour ouvrir.`;
      break;
    case 'abroge': {
      const fin = v.date_fin ? ` depuis le ${formatDate(v.date_fin)}` : '';
      main = `${ref} — ${v.etat_libelle || 'plus en vigueur'}${fin}. Ne pas citer comme droit positif sans vérification. Cliquer pour ouvrir.`;
      break;
    }
    case 'introuvable': {
      const ailleurs = v.existe_dans.length
        ? ` Ce numéro existe dans : ${v.existe_dans.join(', ')} — possible confusion de code.`
        : '';
      main = `${ref} — aucun article de ce numéro trouvé dans ce code : référence peut-être inventée.${ailleurs}`;
      break;
    }
    case 'verifiee':
      main = `${ref} — ${v.detail || verifieeLead(v)}`;
      break;
    case 'incoherente':
    case 'approchante':
    case 'non_retrouvee':
      main = `${ref} — ${v.detail || 'à vérifier'}`;
      break;
    default:
      main = `${ref} — ${v.detail || 'non vérifié'}`;
  }
  if (v.infos?.length) main += `\n${v.infos.join('\n')}`;
  if (v.url && v.kind !== 'citation') main += '\nCliquer pour ouvrir la source.';
  if (v.ancrage === 'hors_sources') return `${main}\n${ANCRAGE_HORS}`;
  if (v.ancrage === 'dans_sources') return `${main}\n${ANCRAGE_DANS}`;
  return main;
}

function verifieeLead(v: CitationVerification): string {
  switch (v.kind) {
    case 'decision':   return 'décision retrouvée dans Judilibre.';
    case 'convention': return 'convention collective retrouvée (Légifrance).';
    case 'citation':   return 'passage retrouvé tel quel.';
    default:           return 'référence retrouvée.';
  }
}

function formatDate(iso: string): string {
  const [y, m, d] = iso.split('-');
  return y && m && d ? `${d}/${m}/${y}` : iso;
}

/**
 * Remplace chaque occurrence vérifiée par un lien Markdown `[texte](#juridix-cite-N)`
 * (N = index dans `report.verifications`). Rendu ensuite par `CitationBadge`.
 *
 * Sûreté : une occurrence n'est annotée que si `text.slice(debut, fin)` vaut
 * exactement `texte` (le message a pu changer depuis la vérification) ; les
 * statuts sans verdict ne produisent aucune annotation.
 */
export function annotateCitations(text: string, report: CitationReport | null): string {
  if (!report) return text;
  const spans: Array<{ debut: number; fin: number; idx: number }> = [];
  report.verifications.forEach((v, idx) => {
    // Sans verdict d'existence, une référence n'est annotée que si elle est hors sources.
    const hasVerdict = v.statut !== 'non_verifiable' && v.statut !== 'erreur';
    if (!hasVerdict && v.ancrage !== 'hors_sources') return;
    for (const o of v.occurrences) {
      if (text.slice(o.debut, o.fin) === o.texte) spans.push({ debut: o.debut, fin: o.fin, idx });
    }
  });
  if (!spans.length) return text;
  spans.sort((a, b) => a.debut - b.debut);
  let out = '';
  let cursor = 0;
  for (const s of spans) {
    if (s.debut < cursor) continue; // chevauchement : on ignore
    out += text.slice(cursor, s.debut);
    out += `[${text.slice(s.debut, s.fin)}](${CITE_HREF_PREFIX}${s.idx})`;
    cursor = s.fin;
  }
  return out + text.slice(cursor);
}

// ── Appel réseau (avec cache de session et dédoublonnage) ─────────────────────

const reportCache = new Map<string, CitationReport | null>();
const inflight = new Map<string, Promise<CitationReport | null>>();
const CACHE_MAX = 200;

/** Signature légère du contexte : le rapport dépend du texte ET de ce que le modèle a reçu. */
function groundingSignature(g: CitationGrounding | null): string {
  if (!g) return '-';
  const total = g.sources.reduce((n, s) => n + s.length, 0);
  return `${g.sources.length}:${total}:${g.keys.length}:${g.keys.join('|').length}`;
}

function cacheKey(text: string, g: CitationGrounding | null, question: string | null = null): string {
  return `${groundingSignature(g)}\u0000${question ?? ''}\u0000${text}`;
}

/** `null` = pas de rapport exploitable (aucune annotation, aucune erreur visible). */
export function verifyCitations(
  text: string,
  grounding: CitationGrounding | null = null,
  signal?: AbortSignal,
  question: string | null = null,
): Promise<CitationReport | null> {
  const key = cacheKey(text, grounding, question);
  if (reportCache.has(key)) return Promise.resolve(reportCache.get(key)!);
  const pending = inflight.get(key);
  if (pending) return pending;

  const p = (async () => {
    try {
      const res = await fetch(`${API_BASE}/api/juridique/verifier-citations`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          ...(grounding
            ? { texte: text, ancrage: true, sources: grounding.sources, cles_sources: grounding.keys }
            : { texte: text }),
          // La question sert à retrouver l'article désigné quand la réponse n'en cite aucun.
          ...(question ? { contexte: question } : {}),
        }),
        signal,
      });
      if (!res.ok) return null; // 503 (PISTE non configuré), 413, 5xx… : silencieux
      const report = (await res.json()) as CitationReport;
      // Ne mémorise que les rapports où rien n'a échoué : un « erreur » doit
      // pouvoir être retenté au prochain affichage.
      if (!report.verifications.some(v => v.statut === 'erreur')) {
        if (reportCache.size >= CACHE_MAX) reportCache.delete(reportCache.keys().next().value as string);
        reportCache.set(key, report);
      }
      return report;
    } catch {
      return null; // hors ligne, annulé…
    } finally {
      inflight.delete(key);
    }
  })();
  inflight.set(key, p);
  return p;
}

/**
 * Vérifie en arrière-plan les références d'un message terminé. N'agit que si
 * `active` (message final, non en cours de génération) ; ne bloque jamais l'affichage.
 * `getGrounding` (optionnel) fournit ce que le modèle a reçu, pour l'ancrage.
 */
export function useCitationReport(
  text: string,
  active: boolean,
  getGrounding?: GroundingGetter,
  getQuestion?: QuestionGetter,
): CitationReport | null {
  const [report, setReport] = useState<CitationReport | null>(null);

  useEffect(() => {
    if (!active || !text || !mightContainCitation(text)) { setReport(null); return; }
    const grounding = getGrounding ? getGrounding() : null;
    const question = getQuestion ? getQuestion() : null;
    const cached = reportCache.get(cacheKey(text, grounding, question));
    if (cached !== undefined) { setReport(cached); return; }
    const ctrl = new AbortController();
    let alive = true;
    verifyCitations(text, grounding, ctrl.signal, question).then(r => { if (alive) setReport(r); });
    return () => { alive = false; ctrl.abort(); };
  }, [text, active, getGrounding, getQuestion]);

  return report;
}
