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
import { getDroitADate, useDroitADate } from './droitADate';

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
 * decision   = décision de justice : Cour de cassation (pourvoi, Judilibre), Conseil d'État et juridictions
 *               administratives (Légifrance), cours d'appel (numéro de RG, Judilibre), ou toute décision citée par ECLI ;
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
  /** Décision retrouvée : identifiant donnant accès à sa fiche (rapprochements, provenance). */
  decision_id?: string | null;
  occurrences: CitationOccurrence[];
}

/** Une version d'un article (Légifrance). Dates « AAAA-MM-JJ ». */
export interface VersionArticle {
  id: string;
  etat: string | null;
  etat_libelle: string | null;
  date_debut: string | null;
  /** `null` = sans fin connue. */
  date_fin: string | null;
  numero: string | null;
}

/** Droit applicable à la date des faits pour un article de code (présent si une date est fixée). */
export interface DroitADate {
  /** Même valeur que `CitationVerification.reference` : sert à l'appariement. */
  reference: string;
  code: string | null;
  numero: string;
  date: string;
  statut: 'applicable' | 'aucune_version' | 'non_verifiable';
  version_applicable: VersionArticle | null;
  /** L'article a changé depuis la date des faits. */
  modifie_depuis: boolean;
  modifications_depuis: VersionArticle[];
  texte_applicable: string | null;
  url_applicable: string | null;
  detail: string | null;
}

export interface CitationReport {
  date: string;
  verifications: CitationVerification[];
  /** Absent quand aucune date des faits n'est fixée. */
  droit_a_date?: DroitADate[];
}

/** Droit à date de la vérification `v`, s'il a été calculé. */
export function droitADateOf(report: CitationReport | null, v: CitationVerification): DroitADate | null {
  return report?.droit_a_date?.find(d => d.reference === v.reference) ?? null;
}

/** Marque « droit dans le temps » : seulement quand le droit applicable diffère du droit actuel. */
export function droitADateBadge(d: DroitADate | null): { icon: string; label: string } | null {
  if (!d) return null;
  if (d.statut === 'aucune_version') return { icon: '🚫', label: "aucune version applicable à cette date" };
  if (d.statut === 'applicable' && d.modifie_depuis) return { icon: '🕘', label: 'modifié depuis la date des faits' };
  return null;
}

/** Lignes d'info-bulle sur le droit applicable à la date des faits. */
export function droitADateLines(d: DroitADate): string[] {
  const date = formatDate(d.date);
  const lines: string[] = [];
  switch (d.statut) {
    case 'applicable': {
      const v = d.version_applicable;
      const debut = v?.date_debut ? ` (en vigueur depuis le ${formatDate(v.date_debut)})` : '';
      lines.push(`Version applicable au ${date}${debut}.`);
      if (d.modifie_depuis) {
        const depuis = d.modifications_depuis
          .filter(m => m.date_debut)
          .map(m => `${formatDate(m.date_debut as string)}${m.etat_libelle ? ` (${m.etat_libelle})` : ''}`)
          .join(', ');
        lines.push(`⚠️ Modifié depuis : ${depuis || 'voir Légifrance'}. La rédaction actuelle peut différer.`);
      } else {
        lines.push('Inchangé depuis cette date.');
      }
      if (d.texte_applicable) lines.push(`« ${d.texte_applicable} »`);
      break;
    }
    case 'aucune_version':
      lines.push(`🚫 ${d.detail || `Aucune version applicable au ${date}.`}`);
      break;
    default:
      lines.push(d.detail || `Droit au ${date} non vérifiable.`);
  }
  return lines;
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
// Abréviations sensibles à la casse (« CE » ≠ « ce », « CA » ≠ « ca ») : un faux positif ne
// coûte qu'un appel inutile, mais autant l'éviter pour les mots les plus courants.
const QUICK_RE_CASSE = /\b(?:CE|CAA|CA|TA)\b[\s,]/;

const QUICK_RE = new RegExp(
  [
    String.raw`\b(?:articles?|arts?)\b\.?\s*\d`,                 // article 1103
    String.raw`\b[LRDA]\s?\.?\s?\d{2,}`,                         // L. 1234-5
    String.raw`\bpourvoi\b|\bcass(?:ation)?\b|\bIDCC\b`,        // décisions, conventions collectives
    String.raw`\bECLI\s?:|Conseil\s+d[’']\s?[ée]tat|cour\s+(?:administrative\s+)?d[’']appel`, // ECLI, juridictions
    String.raw`[«“][^»”]{25,}[»”]|"[^"\n]{25,}"`,                // citations textuelles (« », “ ”, " ")
    // loi / décret / arrêté… suivi (de près) d'un numéro ou d'une date
    String.raw`\b(?:loi|ordonnance|d[ée]cret|arr[êe]t[ée]|circulaire|instruction|d[ée]cision)(?![a-zà-ÿ])[^\n]{0,40}\d`,  // \b ne voit pas le é final (ASCII seulement)
  ].join('|'),
  'i',
);

export function mightContainCitation(text: string): boolean {
  return QUICK_RE.test(text) || QUICK_RE_CASSE.test(text);
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
    // Décision d'une juridiction que Juridix n'interroge pas : visible, sans verdict.
    case 'non_verifiable': return v.kind === 'decision' ? { icon: '⚪', label: 'non vérifiable' } : { icon: '', label: '' };
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
export function citationTitle(v: CitationVerification, report: CitationReport | null = null): string {
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
  const dad = droitADateOf(report, v);
  if (dad) main += `\n${droitADateLines(dad).join('\n')}`;
  if (v.url && v.kind !== 'citation') main += '\nCliquer pour ouvrir la source.';
  if (v.ancrage === 'hors_sources') return `${main}\n${ANCRAGE_HORS}`;
  if (v.ancrage === 'dans_sources') return `${main}\n${ANCRAGE_DANS}`;
  return main;
}

function verifieeLead(v: CitationVerification): string {
  switch (v.kind) {
    case 'decision':   return 'décision retrouvée (la base consultée est indiquée ci-dessous).';
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
    const hasVerdict = (v.statut !== 'non_verifiable' && v.statut !== 'erreur')
      || (v.kind === 'decision' && v.statut === 'non_verifiable'); // visible : jamais présentée comme vérifiée
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
  // La date des faits change le rapport (versions applicables) : elle fait partie de la clé.
  return `${groundingSignature(g)}\u0000${question ?? ''}\u0000${getDroitADate() ?? ''}\u0000${text}`;
}

/** `null` = pas de rapport exploitable (aucune annotation, aucune erreur visible). */
// Plafond de vérifications simultanées : un changement de date des faits relance la
// vérification de tous les messages de la conversation, chacune pouvant déclencher
// plusieurs appels Légifrance côté serveur. Sans file d'attente, une longue
// conversation saturerait l'application.
const MAX_VERIFICATIONS_SIMULTANEES = 3;
let verificationsEnCours = 0;
const fileAttente: Array<() => void> = [];

async function prendreCreneau(): Promise<void> {
  if (verificationsEnCours < MAX_VERIFICATIONS_SIMULTANEES) { verificationsEnCours++; return; }
  await new Promise<void>(resolve => fileAttente.push(resolve)); // le créneau est transmis tel quel
}

function libererCreneau(): void {
  const suivant = fileAttente.shift();
  if (suivant) suivant(); else verificationsEnCours--;
}

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
    await prendreCreneau();
    try {
      if (signal?.aborted) return null; // annulé pendant l'attente (conversation changée, date modifiée…)
      const res = await fetch(`${API_BASE}/api/juridique/verifier-citations`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          ...(grounding
            ? { texte: text, ancrage: true, sources: grounding.sources, cles_sources: grounding.keys }
            : { texte: text }),
          // La question sert à retrouver l'article désigné quand la réponse n'en cite aucun.
          ...(question ? { contexte: question } : {}),
          // Date des faits : versions applicables et modifications depuis (voir `droit_a_date`).
          ...(getDroitADate() ? { date_applicable: getDroitADate() } : {}),
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
      libererCreneau();
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
  const dateFaits = useDroitADate(); // relance la vérification quand la date des faits change

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
  }, [text, active, getGrounding, getQuestion, dateFaits]);

  return report;
}


// ── Fiche d'une décision : rapprochements et provenance ──────────────────────
// Reflète `GET /api/juridique/decision/:id/fiche` (jurisprudence.rs). Rien n'y est
// déduit : chaque rapprochement porte la base et le champ d'où il vient, et ce que les API
// ne fournissent pas (« cité par », revirements) est dit plutôt que passé sous silence.

export type NiveauDecision = 'cassation' | 'fond' | 'inconnu';
export type RelationDecision = 'decision_attaquee' | 'decision_suivante' | 'rapprochement' | 'lien_legifrance';

export interface FicheProvenance { source: string; champ: string }

export interface FicheRapprochement {
  relation: RelationDecision;
  libelle: string;
  cible: 'decision' | 'texte' | 'inconnue';
  id: string | null;
  titre: string | null;
  numero: string | null;
  date: string | null;
  juridiction: string | null;
  siege: string | null;
  chambre: string | null;
  solution: string | null;
  description: string | null;
  themes: string[];
  url: string | null;
  niveau: NiveauDecision;
  chronologie: 'anterieure' | 'meme_jour' | 'posterieure' | null;
  type_lien: string | null;
  sens: string | null;
  provenance: FicheProvenance;
}

export interface FicheDecision {
  id: string;
  source: string;
  titre: string | null;
  juridiction: string | null;
  niveau: NiveauDecision;
  niveau_libelle: string;
  chambre: string | null;
  formation: string | null;
  numero: string | null;
  ecli: string | null;
  date: string | null;
  solution: string | null;
  publication: string[];
  interet_particulier: boolean | null;
  partielle: boolean;
  visas: { titre: string; url: string | null }[];
  rapprochements: FicheRapprochement[];
  dates_cles: { libelle: string; date: string | null; provenance: FicheProvenance }[];
  indisponibles: { element: string; raison: string }[];
}

const ficheCache = new Map<string, FicheDecision>();

/** Fiche d'une décision, `null` si la source est indisponible (jamais bloquant). */
export async function fetchFiche(id: string, signal?: AbortSignal): Promise<FicheDecision | null> {
  const known = ficheCache.get(id);
  if (known) return known;
  try {
    const res = await fetch(`${API_BASE}/api/juridique/decision/${encodeURIComponent(id)}/fiche`, { signal });
    if (!res.ok) return null;
    const fiche = (await res.json()) as FicheDecision;
    ficheCache.set(id, fiche);
    return fiche;
  } catch {
    return null;
  }
}

const CHRONOLOGIE: Record<string, string> = {
  anterieure: 'antérieure', meme_jour: 'même jour', posterieure: 'postérieure',
};

/** Ligne lisible d'un rapprochement, provenance comprise. */
export function rapprochementLine(r: FicheRapprochement): string {
  const identite = [r.titre, r.numero ? `n° ${r.numero}` : null, r.date ? formatDate(r.date) : null]
    .filter(Boolean).join(' — ');
  const lieu = [r.siege ?? r.juridiction, r.chambre].filter(Boolean).join(', ');
  const lien = r.type_lien ? ` (type « ${r.type_lien} »${r.sens ? `, sens « ${r.sens} »` : ''})` : '';
  const chrono = r.chronologie ? ` — décision ${CHRONOLOGIE[r.chronologie]}` : '';
  const corps = [identite, lieu].filter(Boolean).join(' · ') || 'sans intitulé';
  return `${r.libelle}${lien} : ${corps}${chrono} [${r.provenance.source}, ${r.provenance.champ}]`;
}

/** Contenu de la fiche en lignes : identité, rapprochements, ce que les API ne fournissent pas. */
export function ficheLines(f: FicheDecision): { identite: string[]; rapprochements: string[]; limites: string[] } {
  const identite = [
    `${f.niveau_libelle}${f.juridiction && f.niveau !== 'cassation' ? ` — ${f.juridiction}` : ''}`,
    [f.chambre, f.formation].filter(Boolean).join(', '),
    f.date ? `Décision du ${formatDate(f.date)}` : '',
    f.solution ? `Solution : ${f.solution}` : '',
    f.partielle ? 'Décision publiée par extraits.' : '',
    `Source : ${f.source}`,
  ].filter(Boolean);
  return {
    identite,
    rapprochements: f.rapprochements.map(rapprochementLine),
    limites: f.indisponibles.map(i => i.raison),
  };
}
