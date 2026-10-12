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

// Synthèse de veille : à partir de résultats sélectionnés dans une veille,
//   1. charge le texte intégral de chacun (mêmes routes et mêmes extracteurs que la
//      visionneuse de documents) ;
//   2. demande au backend le prompt « note de veille » (POST /api/juridique/envoyer-chat,
//      mode `veille_synthese`) ;
//   3. (le panneau envoie ce prompt au chat, puis reçoit la synthèse) ;
//   4. assemble la note finale en Markdown — en-tête, synthèse, liste des textes
//      analysés — que `generateDocx` convertit en fichier Word.
//
// La liste des textes analysés est produite ICI, de façon déterministe, et non par le
// modèle : elle reste exacte et vérifiable quoi que le LLM ait écrit.

import { API_BASE } from '../constants';
import { extractTextContent } from '../components/juridique/content';
import { htmlToPlainBlocks, isTextId } from '../components/juridique/content/html-helpers';
import { fondToEndpointInfo } from '../components/juridique/hooks/useDocumentViewer';
import { openArgs, formatTexteDate, type VeilleResultat } from './veille';

/** Nombre maximal de textes par synthèse. */
export const MAX_TEXTES = 15;
/** Plafonds de caractères transmis au modèle (≈ 4 caractères par jeton). */
export const MAX_CHARS_PAR_TEXTE = 15_000;
export const MAX_CHARS_TOTAL = 90_000;
/** Chargements simultanés (les API PISTE sont limitées en débit). */
const CONCURRENCE = 3;

export interface TexteCharge {
  resultat: VeilleResultat;
  /** Type de document reconnu par la visionneuse (loda, juri, decision…). */
  type: string;
  contenu: string;
  /** Vrai si le texte a été coupé pour tenir dans le budget transmis au modèle. */
  tronque: boolean;
  /** Renseigné si le texte n'a pas pu être chargé : il n'est alors pas analysé. */
  erreur?: string;
}

/** Coupe `texte` à `max` caractères, de préférence en fin de paragraphe ou de phrase. */
export function tronquer(texte: string, max: number): { texte: string; tronque: boolean } {
  if (texte.length <= max) return { texte, tronque: false };
  const fenetre = texte.slice(0, max);
  const coupe = Math.max(fenetre.lastIndexOf('\n\n'), fenetre.lastIndexOf('. '), fenetre.lastIndexOf('\n'));
  const garde = coupe > max * 0.6 ? fenetre.slice(0, coupe + 1) : fenetre;
  const omis = texte.length - garde.length;
  return {
    texte: `${garde.trimEnd()}\n\n[… texte tronqué : ${omis.toLocaleString('fr-FR')} caractères non transmis]`,
    tronque: true,
  };
}

/** Budget de caractères par texte pour `n` textes (le total reste borné). */
export function budgetParTexte(n: number): number {
  return Math.max(2_000, Math.min(MAX_CHARS_PAR_TEXTE, Math.floor(MAX_CHARS_TOTAL / Math.max(1, n))));
}

async function chargerTexte(r: VeilleResultat, budget: number, signal?: AbortSignal): Promise<TexteCharge> {
  const base: TexteCharge = { resultat: r, type: '', contenu: '', tronque: false };
  try {
    const { fond, source } = openArgs(r);
    const id = r.texte_id;
    let endpoint: string;
    let type: string;
    if (source === 'judilibre') {
      endpoint = `${API_BASE}/api/juridique/decision/${encodeURIComponent(id)}?resolve_references=true`;
      type = 'decision';
    } else {
      const info = fondToEndpointInfo(fond, id, isTextId(id));
      endpoint = info.endpoint;
      type = info.type;
    }
    const res = await fetch(endpoint, { signal });
    if (!res.ok) {
      const e = await res.json().catch(() => ({}));
      throw new Error((e as { error?: string }).error || `Erreur ${res.status}`);
    }
    const raw = await res.json();
    if (raw?.error) throw new Error(String(raw.error));

    const content = extractTextContent(raw, type, r.titre);
    const brut = htmlToPlainBlocks(content.body).trim();
    if (!brut) throw new Error('contenu vide');
    const { texte, tronque } = tronquer(brut, budget);
    return { ...base, type, contenu: texte, tronque };
  } catch (e) {
    if ((e as { name?: string }).name === 'AbortError') throw e;
    return { ...base, erreur: e instanceof Error ? e.message : String(e) };
  }
}

/** Charge les textes (3 en parallèle), dans l'ordre des résultats fournis. */
export async function chargerTextes(
  resultats: VeilleResultat[],
  onProgress?: (faits: number, total: number) => void,
  signal?: AbortSignal,
): Promise<TexteCharge[]> {
  const budget = budgetParTexte(resultats.length);
  const sortie: TexteCharge[] = new Array(resultats.length);
  let suivant = 0;
  let faits = 0;
  const worker = async () => {
    while (suivant < resultats.length) {
      const i = suivant++;
      sortie[i] = await chargerTexte(resultats[i], budget, signal);
      onProgress?.(++faits, resultats.length);
    }
  };
  await Promise.all(Array.from({ length: Math.min(CONCURRENCE, resultats.length) }, worker));
  return sortie;
}

/** Demande au backend le prompt « note de veille » pour les textes chargés. */
export async function demanderPrompt(
  nomVeille: string,
  consigne: string,
  textes: TexteCharge[],
  winnowEnabled: boolean,
): Promise<string> {
  const c = consigne.trim();
  const res = await fetch(`${API_BASE}/api/juridique/envoyer-chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      mode: 'veille_synthese',
      titre: nomVeille,
      prompt_custom: c || undefined,
      focus: c || undefined,
      winnow_enabled: winnowEnabled,
      dossier: textes.map((t) => ({
        titre: t.resultat.titre,
        source: t.resultat.source,
        type: t.type,
        contenu: t.contenu,
      })),
    }),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok || data.error || !data.message) {
    throw new Error((data as { error?: string }).error || `Erreur ${res.status}`);
  }
  return data.message as string;
}

// ── Note finale ──────────────────────────────────────────────────────────────

const SOURCES: Record<string, string> = { legifrance: 'Légifrance', judilibre: 'Judilibre' };

function dateFr(d: Date): string {
  return d.toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: 'numeric' });
}

/**
 * Une puce de la liste des textes : « Titre — nature · date · source ». Puces et non
 * numéros : `generateDocx` partage un seul compteur de numérotation dans tout le document,
 * qui continuerait celui d'une éventuelle liste numérotée de la synthèse.
 */
function ligneReference(t: TexteCharge): string {
  const r = t.resultat;
  const meta = [r.nature, formatTexteDate(r.date_texte), SOURCES[r.source] ?? r.source].filter(Boolean).join(' · ');
  return `- ${r.titre}${meta ? ` — ${meta}` : ''}`;
}

/**
 * Assemble la note de veille en Markdown (prête pour l'aperçu et pour `generateDocx`).
 * `synthese` est la réponse du modèle ; `textes` tous les textes sélectionnés, chargés ou non.
 */
export function assemblerNote(nomVeille: string, synthese: string, textes: TexteCharge[], date: Date = new Date()): string {
  const analyses = textes.filter((t) => !t.erreur);
  const echecs = textes.filter((t) => t.erreur);
  const tronques = analyses.filter((t) => t.tronque);

  const out: string[] = [
    '# Note de veille juridique',
    '',
    `**Veille :** ${nomVeille}`,
    '',
    `**Date :** ${dateFr(date)}`,
    '',
    `**Textes analysés :** ${analyses.length}`,
    '',
    '---',
    '',
    synthese.trim(),
    '',
    '---',
    '',
    '## Textes analysés',
    '',
    ...analyses.map(ligneReference),
  ];
  if (tronques.length > 0) {
    out.push('', '**Textes tronqués** (seule la première partie a été transmise au modèle) :', '',
      ...tronques.map((t) => `- ${t.resultat.titre}`));
  }
  if (echecs.length > 0) {
    out.push('', '**Textes non chargés** (non pris en compte dans la synthèse) :', '',
      ...echecs.map((t) => `- ${t.resultat.titre} — ${t.erreur}`));
  }
  out.push('', '*Note générée par un assistant d\'IA à partir des textes cités : à vérifier avant tout usage.*');
  return out.join('\n');
}

/** Nom de fichier : `veille-<nom>-AAAA-MM-JJ.docx`. */
export function nomFichierNote(nomVeille: string, date: Date = new Date()): string {
  const slug = nomVeille
    .normalize('NFD').replace(/[\u0300-\u036f]/g, '')
    .toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'note';
  const iso = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  return `veille-${slug}-${iso}.docx`;
}
