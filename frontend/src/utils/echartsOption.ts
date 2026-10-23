// ============================================================================
// Juridix — Assistant IA juridique desktop
// ============================================================================
// Analyse SÛRE des options ECharts produites par le LLM (correctif E-01/E-02 de
// l'audit de sécurité).
//
// Avant : `new Function('return (' + code + ')')()` — le code d'un bloc ```echarts
// (donc influençable par un document ou un résultat d'outil piégé) était exécuté
// comme du JavaScript dans le WebView privilégié.
// Désormais : le texte est converti en JSON strict puis analysé par `JSON.parse`,
// qui ne peut JAMAIS exécuter de code. L'arbre obtenu est ensuite validé
// (profondeur, taille, clés dangereuses) et ses chaînes sont débarrassées de
// toute balise HTML (les infobulles ECharts rendent du HTML).
// ============================================================================

import { cleanEChartsCode } from './text';

const MAX_DEPTH = 24;
const MAX_NODES = 100_000;
const FORBIDDEN_KEYS = new Set(['__proto__', 'constructor', 'prototype']);

/**
 * Met entre guillemets les clés d'objet nues (`{ title: ... }` → `{ "title": ... }`)
 * et convertit `undefined` en `null`, hors des chaînes. Tolérance de syntaxe
 * uniquement : le résultat est de toute façon passé à JSON.parse.
 */
function toStrictJson(src: string): string {
  let out = '';
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === '\\' ? 2 : 1;
      out += src.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (/[A-Za-z_$]/.test(c)) {
      let j = i + 1;
      while (j < n && /[A-Za-z0-9_$]/.test(src[j])) j++;
      const word = src.slice(i, j);
      let k = j;
      while (k < n && /\s/.test(src[k])) k++;
      const prev = out.trimEnd().slice(-1);
      if (src[k] === ':' && (prev === '{' || prev === ',')) out += `"${word}"`;
      else if (word === 'undefined') out += 'null';
      else out += word;
      i = j;
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

/** Supprime les balises HTML des chaînes et rejette les structures abusives. */
function sanitizeTree(value: unknown, depth: number, counter: { n: number }): unknown {
  if (++counter.n > MAX_NODES) throw new Error('option ECharts trop volumineuse');
  if (depth > MAX_DEPTH) throw new Error('option ECharts trop imbriquée');
  if (typeof value === 'string') return value.replace(/<[^>]*>/g, '');
  if (value === null || typeof value === 'number' || typeof value === 'boolean') return value;
  if (Array.isArray(value)) return value.map((v) => sanitizeTree(v, depth + 1, counter));
  if (typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (FORBIDDEN_KEYS.has(k)) continue;
      out[k] = sanitizeTree(v, depth + 1, counter);
    }
    return out;
  }
  throw new Error('valeur non autorisée dans l\'option ECharts');
}

/**
 * Analyse le contenu d'un bloc ```echarts. Lève une Error si le contenu n'est pas du
 * JSON (même tolérant) — jamais d'exécution de code.
 */
export function parseEChartsOption(code: string): Record<string, unknown> {
  const cleaned = toStrictJson(cleanEChartsCode(code));
  const parsed: unknown = JSON.parse(cleaned);
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('l\'option ECharts doit être un objet JSON');
  }
  return sanitizeTree(parsed, 0, { n: 0 }) as Record<string, unknown>;
}
