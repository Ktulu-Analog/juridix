// ============================================================================
// Juridix — Veille juridique : types, appels API, fonctions pures
// ============================================================================
// Côté frontend du chantier 8. Le backend (veille.rs) rejoue périodiquement les
// recherches enregistrées et mémorise ce qui est nouveau ; ici on affiche, on
// configure et on ouvre les textes détectés.
import { useCallback, useEffect, useRef, useState } from 'react';
import { API_BASE } from '../constants';
import type { Fond, SearchResult, Source } from '../components/juridique/types';

export type VeilleSource = 'legifrance' | 'judilibre';

export interface VeilleCriteres {
  source: VeilleSource;
  query: string;
  fond?: string | null;
  natures: string[];
  code?: string | null;
  juridictions: string[];
}

export interface Veille {
  id: string;
  nom: string;
  criteres: VeilleCriteres;
  active: boolean;
  frequence_heures: number;
  depuis: string;
  created_at: string;
  last_run_at: string | null;
  last_success_at: string | null;
  last_error: string | null;
  last_notice: string | null;
  non_lus: number;
  total: number;
}

export interface VeilleResultat {
  id: number;
  veille_id: string;
  texte_id: string;
  source: VeilleSource;
  fond: string;
  titre: string;
  nature: string | null;
  date_texte: string | null;
  detecte_le: string;
  vu: boolean;
}

export interface VeilleLimites {
  frequence_min_h: number;
  frequence_max_h: number;
  frequence_defaut_h: number;
  max_veilles: number;
  fonds: string[];
  recouvrement_jours: number;
}

export interface RunReport {
  veille_id: string;
  nouveaux: number;
  trouves: number;
  truncated: boolean;
  error: string | null;
}

export interface VeilleInput {
  nom: string;
  criteres: VeilleCriteres;
  active?: boolean;
  frequence_heures?: number;
  depuis?: string;
}

// ── Libellés ──────────────────────────────────────────────────────────────────

export const FOND_OPTIONS: { value: string; label: string }[] = [
  { value: 'LODA_DATE', label: 'Lois, décrets, arrêtés (textes en vigueur)' },
  { value: 'JORF',      label: 'Journal officiel' },
  { value: 'CODE_DATE', label: 'Codes — nouvelles versions d\'articles' },
  { value: 'JURI',      label: 'Jurisprudence judiciaire (Légifrance)' },
  { value: 'CETAT',     label: 'Conseil d\'État' },
  { value: 'CONSTIT',   label: 'Conseil constitutionnel' },
  { value: 'CIRC',      label: 'Circulaires et instructions' },
  { value: 'CNIL',      label: 'CNIL' },
  { value: 'KALI',      label: 'Conventions collectives' },
  { value: 'ACCO',      label: 'Accords d\'entreprise' },
];

/** Fonds qui acceptent un filtre sur la nature du texte. */
export const FONDS_AVEC_NATURE = ['LODA_DATE', 'JORF'];

export const NATURES: { value: string; label: string }[] = [
  { value: 'LOI', label: 'Loi' },
  { value: 'ORDONNANCE', label: 'Ordonnance' },
  { value: 'DECRET', label: 'Décret' },
  { value: 'ARRETE', label: 'Arrêté' },
  { value: 'DECISION', label: 'Décision' },
];

export const JURIDICTIONS: { value: string; label: string }[] = [
  { value: 'cc', label: 'Cour de cassation' },
  { value: 'ca', label: 'Cours d\'appel' },
  { value: 'tj', label: 'Tribunaux judiciaires' },
];

export const FREQUENCES: { value: number; label: string }[] = [
  { value: 6,   label: 'Toutes les 6 heures' },
  { value: 12,  label: 'Deux fois par jour' },
  { value: 24,  label: 'Une fois par jour' },
  { value: 72,  label: 'Tous les 3 jours' },
  { value: 168, label: 'Une fois par semaine' },
];

export const CODES_VEILLE = [
  'Code civil', 'Code du travail', 'Code pénal', 'Code de procédure civile', 'Code de procédure pénale',
  'Code de commerce', 'Code de la consommation', 'Code général des impôts', 'Code de la sécurité sociale',
  'Code de l\'urbanisme', 'Code de la construction et de l\'habitation', 'Code monétaire et financier',
  'Code de justice administrative', 'Code de l\'environnement', 'Code de la propriété intellectuelle',
  'Code de la santé publique', 'Code des assurances', 'Code général de la fonction publique',
];

const fondLabel = (v?: string | null) => FOND_OPTIONS.find(f => f.value === v)?.label ?? v ?? '';

/** Résumé d'une veille en une ligne : « Lois, décrets, arrêtés · « télétravail » · Décret ». */
export function describeCriteres(c: VeilleCriteres): string {
  const parts: string[] = [];
  if (c.source === 'judilibre') {
    const j = c.juridictions.length
      ? c.juridictions.map(v => JURIDICTIONS.find(x => x.value === v)?.label ?? v).join(', ')
      : 'Cour de cassation';
    parts.push(`Judilibre — ${j}`);
  } else {
    parts.push(fondLabel(c.fond));
    if (c.code) parts.push(c.code);
  }
  parts.push(`« ${c.query} »`);
  if (c.natures.length) parts.push(c.natures.map(n => NATURES.find(x => x.value === n)?.label ?? n).join(', '));
  return parts.join(' · ');
}

/** « il y a 3 h », « hier », « le 12/09/2026 » — jamais d'heure absolue ambiguë. */
export function relativeTime(iso: string | null, now: number = Date.now()): string {
  if (!iso) return 'jamais';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return 'jamais';
  const min = Math.max(0, Math.round((now - t) / 60000));
  if (min < 1) return 'à l\'instant';
  if (min < 60) return `il y a ${min} min`;
  const h = Math.round(min / 60);
  if (h < 24) return `il y a ${h} h`;
  const days = Math.round(h / 24);
  if (days === 1) return 'hier';
  if (days < 14) return `il y a ${days} jours`;
  return `le ${new Date(t).toLocaleDateString('fr-FR')}`;
}

/** « 2026-09-30 » → « 30/09/2026 ». */
export function formatTexteDate(d: string | null): string {
  const m = d ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(d) : null;
  return m ? `${m[3]}/${m[2]}/${m[1]}` : '';
}

// ── Ouverture d'un résultat dans la visionneuse ───────────────────────────────

/** Fonds dont l'ouverture se fait par préfixe d'identifiant (comme un lien de citation). */
const ROUTES_PAR_PREFIXE = ['LODA_DATE', 'CODE_DATE', 'JORF'];

/** Arguments pour `openDocument(result, true, fond, source)` de la visionneuse. */
export function openArgs(r: VeilleResultat): { result: SearchResult; fond: Fond; source: Source } {
  return {
    result: { id: r.texte_id, title: r.titre, titles: [{ id: r.texte_id, title: r.titre }] } as SearchResult,
    fond: (ROUTES_PAR_PREFIXE.includes(r.fond) ? 'ALL' : r.fond) as Fond,
    source: r.source,
  };
}

// ── API ───────────────────────────────────────────────────────────────────────

const BASE = `${API_BASE}/api/veilles`;

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((data as { error?: string }).error || `Erreur ${res.status}`);
  return data as T;
}

export const veilleApi = {
  list: () => request<{ veilles: Veille[]; limites: VeilleLimites }>('GET', ''),
  create: (i: VeilleInput) => request<Veille>('POST', '', i),
  update: (id: string, i: VeilleInput) => request<Veille>('PUT', `/${encodeURIComponent(id)}`, i),
  remove: (id: string) => request<{ ok: boolean }>('DELETE', `/${encodeURIComponent(id)}`),
  resultats: (id: string, opts: { nonLus?: boolean; limit?: number; offset?: number } = {}) => {
    const q = new URLSearchParams();
    if (opts.nonLus) q.set('non_lus', 'true');
    if (opts.limit) q.set('limit', String(opts.limit));
    if (opts.offset) q.set('offset', String(opts.offset));
    const qs = q.toString();
    return request<{ resultats: VeilleResultat[]; total: number; non_lus: number }>('GET', `/${encodeURIComponent(id)}/resultats${qs ? `?${qs}` : ''}`);
  },
  marquerLus: (id: string, ids?: number[]) =>
    request<{ marques: number }>('POST', `/${encodeURIComponent(id)}/resultats/lu`, ids ? { ids } : {}),
  executer: (id: string) => request<RunReport>('POST', `/${encodeURIComponent(id)}/executer`),
  nonLus: () => request<{ total: number; par_veille: Record<string, number> }>('GET', '/non-lus'),
};

/** Message d'accueil de la création selon le résultat de la première exécution. */
export function summarizeRun(r: RunReport): string {
  if (r.error) return r.error;
  const base = r.nouveaux === 0 ? 'Aucun nouveau résultat.' : r.nouveaux === 1 ? '1 nouveau résultat.' : `${r.nouveaux} nouveaux résultats.`;
  return r.truncated ? `${base} Résultats nombreux : précisez vos critères.` : base;
}

/**
 * Nombre de résultats non lus (badge), relevé toutes les `intervalMs`. Silencieux si le
 * backend est indisponible : la veille ne doit jamais gêner le reste de l'application.
 */
export function useVeilleBadge(intervalMs = 60_000): { total: number; refresh: () => void } {
  const [total, setTotal] = useState(0);
  const alive = useRef(true);
  const refresh = useCallback(() => {
    veilleApi.nonLus().then(r => { if (alive.current) setTotal(r.total); }).catch(() => { /* backend pas prêt */ });
  }, []);
  useEffect(() => {
    alive.current = true;
    refresh();
    const t = setInterval(refresh, intervalMs);
    return () => { alive.current = false; clearInterval(t); };
  }, [refresh, intervalMs]);
  return { total, refresh };
}
