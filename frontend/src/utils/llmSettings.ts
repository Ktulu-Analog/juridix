// Réglages LLM non sensibles lus hors des composants React (ex. dans `useChatStream`).
// Les secrets (bearer, clés) ne passent jamais par ici : voir secureStorage.ts.

/** Choix proposés dans Réglages → « Longueur maximale des réponses ». `valeur` 0 = automatique. */
export const LONGUEURS_REPONSE: { valeur: number; label: string }[] = [
  { valeur: 0,      label: 'Automatique (valeur du serveur)' },
  { valeur: 4_000,  label: 'Courte — environ 3 000 mots' },
  { valeur: 16_000, label: 'Standard — environ 11 000 mots' },
  { valeur: 32_000, label: 'Longue — environ 22 000 mots' },
  { valeur: 64_000, label: 'Très longue — environ 45 000 mots' },
];

/** Plafond de tokens de réponse choisi par l'utilisateur, ou `undefined` (= automatique). */
export function readLlmMaxTokens(): number | undefined {
  try {
    const saved = JSON.parse(localStorage.getItem('juridix_settings') || '{}');
    const n = Number(saved?.llm_max_tokens);
    return Number.isFinite(n) && n > 0 ? Math.floor(n) : undefined;
  } catch {
    return undefined;
  }
}
