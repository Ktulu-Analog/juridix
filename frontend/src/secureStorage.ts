// ============================================================================
// Juridix — Assistant IA desktop
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
//
// Stockage sécurisé des secrets — côté frontend.
//
// Les trois champs sensibles de `Settings` (bearer LLM, clé API Qdrant,
// secret client PISTE) ne sont plus sérialisés dans le blob JSON
// `juridix_settings` de `localStorage` : ils passent par les commandes Tauri
// `store_secret`/`read_secret`/`delete_secret` (voir src-tauri/src/secrets.rs
// et lib.rs), qui les écrivent dans le trousseau du système d'exploitation.
//
// Repli navigateur : UNIQUEMENT si Tauri n'est pas disponible (mode développement
// web, `npm run dev` hors de l'app desktop), on retombe sur `localStorage` sous
// une clé dédiée — moins sûr, mais fonctionnel, avec un avertissement en
// console pour que ça ne passe pas inaperçu en développement.
//
// Dans l'application desktop, il n'y a AUCUN repli en clair : si le trousseau
// système échoue (verrouillé, absent — fréquent sous Linux sans Secret Service),
// le secret reste en mémoire pour la session seulement (jamais écrit sur le
// disque) et l'utilisateur en est averti (`getSecretStorageIssue`, bannière dans
// l'application). Il devra le ressaisir au prochain lancement.
// ============================================================================

/** Champs de `Settings` déplacés vers le stockage sécurisé. */
export const SECRET_KEYS = ['bearer', 'qdrant_api_key', 'piste_client_secret'] as const;
export type SecretKey = (typeof SECRET_KEYS)[number];

type TauriInvoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

function tauriInvoke(): TauriInvoke | undefined {
  return (window as any).__TAURI__?.core?.invoke;
}

let secretIssue: string | null = null;
const ISSUE_EVENT = 'juridix:secrets-issue';

function reportIssue(message: string) {
  secretIssue = message;
  window.dispatchEvent(new CustomEvent(ISSUE_EVENT, { detail: message }));
}

/** Problème de stockage des secrets signalé pendant cette session (null si aucun). */
export function getSecretStorageIssue(): string | null {
  return secretIssue;
}

/** S'abonne aux signalements ; renvoie la fonction de désabonnement. */
export function onSecretStorageIssue(cb: (message: string) => void): () => void {
  const handler = (e: Event) => cb(String((e as CustomEvent).detail ?? ''));
  window.addEventListener(ISSUE_EVENT, handler);
  return () => window.removeEventListener(ISSUE_EVENT, handler);
}

const ISSUE_MESSAGE =
  'Le trousseau du système est indisponible : vos identifiants (jeton LLM, clé Qdrant, secret PISTE) ' +
  'ne sont pas enregistrés sur ce poste. Ils restent en mémoire pour cette session et devront être ' +
  'ressaisis au prochain lancement.';

let warnedNoTauri = false;
function warnFallbackOnce() {
  if (warnedNoTauri) return;
  warnedNoTauri = true;
  // eslint-disable-next-line no-console
  console.warn(
    '[secrets] Trousseau système indisponible (hors app desktop Tauri) — ' +
    'repli sur localStorage pour cette session. Les identifiants sont alors ' +
    'stockés en clair, comme avant ce chantier. Voir docs/ROADMAP.md, chantier 1.'
  );
}

const FALLBACK_PREFIX = 'juridix_secret_';

async function storeSecret(key: SecretKey, value: string): Promise<void> {
  const invoke = tauriInvoke();
  if (invoke) {
    try {
      await invoke<void>('store_secret', { key, value });
      return;
    } catch (e) {
      // Pas de repli en clair dans l'application : voir l'en-tête du fichier.
      console.error(`[secrets] store_secret("${key}") a échoué`, e);
      reportIssue(ISSUE_MESSAGE);
      return;
    }
  }
  warnFallbackOnce();
  localStorage.setItem(FALLBACK_PREFIX + key, value);
}

async function readSecret(key: SecretKey): Promise<string | undefined> {
  const invoke = tauriInvoke();
  if (invoke) {
    try {
      const v = await invoke<string | null>('read_secret', { key });
      return v ?? undefined;
    } catch (e) {
      console.error(`[secrets] read_secret("${key}") a échoué`, e);
      reportIssue(ISSUE_MESSAGE);
      return undefined;
    }
  }
  warnFallbackOnce();
  return localStorage.getItem(FALLBACK_PREFIX + key) ?? undefined;
}

async function deleteSecret(key: SecretKey): Promise<void> {
  const invoke = tauriInvoke();
  if (invoke) {
    try {
      await invoke<void>('delete_secret', { key });
    } catch (e) {
      console.error(`[secrets] delete_secret("${key}") a échoué`, e);
    }
  }
  // Toujours nettoyer le repli local aussi, migration ou pas : évite qu'une
  // ancienne valeur y traîne indéfiniment une fois le trousseau opérationnel.
  localStorage.removeItem(FALLBACK_PREFIX + key);
}

/**
 * Charge `bearer`/`qdrant_api_key`/`piste_client_secret` depuis le stockage
 * sécurisé et les fusionne dans `settings`. Migration à sens unique et
 * silencieuse : si l'objet passé en argument contient encore l'une de ces
 * valeurs en clair (installation antérieure à ce chantier, lue depuis
 * l'ancien blob `juridix_settings`), elle est copiée vers le stockage
 * sécurisé une bonne fois pour toutes.
 *
 * À appeler une fois, au démarrage de l'application, avant tout appel réseau
 * qui dépendrait de ces identifiants.
 */
export async function loadSecretsIntoSettings<S extends Partial<Record<SecretKey, string>>>(
  settings: S
): Promise<S> {
  const merged: S = { ...settings };
  for (const key of SECRET_KEYS) {
    const legacyValue =
      typeof settings[key] === 'string' && (settings[key] as string).trim() !== ''
        ? (settings[key] as string)
        : undefined;

    let value = await readSecret(key);
    if (!value) {
      // Résidu éventuel d'un ancien repli en clair (versions antérieures, trousseau défaillant) :
      // migré vers le trousseau puis supprimé du WebView. Jamais conservé en clair dans l'app.
      const residue = tauriInvoke() ? localStorage.getItem(FALLBACK_PREFIX + key) || undefined : undefined;
      const candidate = legacyValue ?? residue;
      if (candidate) {
        await storeSecret(key, candidate);
        value = candidate;
        // eslint-disable-next-line no-console
        console.info(`[secrets] ${key} migré vers le stockage sécurisé.`);
      }
    }
    if (tauriInvoke()) localStorage.removeItem(FALLBACK_PREFIX + key);
    merged[key] = (value ?? '') as S[SecretKey];
  }
  return merged;
}

/**
 * Écrit les trois secrets de `settings` dans le stockage sécurisé (jamais
 * dans `localStorage`). Un champ vidé par l'utilisateur dans les Réglages
 * supprime l'entrée correspondante plutôt que d'y stocker une chaîne vide.
 * À appeler à chaque sauvegarde des Réglages, en parallèle de la persistance
 * du reste de `Settings`.
 */
export async function persistSecrets<S extends Partial<Record<SecretKey, string>>>(settings: S): Promise<void> {
  await Promise.all(
    SECRET_KEYS.map(async (key) => {
      const value = typeof settings[key] === 'string' ? (settings[key] as string).trim() : '';
      if (value) await storeSecret(key, value);
      else await deleteSecret(key);
    })
  );
}

/**
 * Retire les champs secrets d'une copie de `settings` — à utiliser
 * systématiquement avant tout `JSON.stringify` destiné à `localStorage`
 * (juridix_settings), pour ne plus jamais les y écrire en clair.
 */
export function stripSecrets<S extends Partial<Record<SecretKey, string>>>(settings: S): S {
  const out: S = { ...settings };
  for (const key of SECRET_KEYS) delete out[key];
  return out;
}
