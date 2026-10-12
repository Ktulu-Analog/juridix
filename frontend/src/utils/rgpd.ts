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

// Export et effacement des données de l'utilisateur (RGPD) — côté frontend.
// Toute la logique sensible est dans les commandes Tauri (src-tauri/src/rgpd.rs) ; elles sont
// volontairement des commandes IPC et non des routes HTTP, donc hors de portée de l'API locale.

/* eslint-disable @typescript-eslint/no-explicit-any */

export interface ExportReport { path: string; conversations: number; fichiers: number; chiffree: boolean; }
export interface BackupStatus { exists: boolean; path?: string; sizeBytes?: number; modified?: string; }
export interface WipeReport { fichiersSupprimes: string[]; avertissements: string[]; redemarrageRequis: boolean; }

/** Mot à saisir pour confirmer l'effacement (revérifié côté Rust). */
export const WIPE_WORD = 'EFFACER';

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const f = (window as any).__TAURI__?.core?.invoke;
  if (!f) return Promise.reject(new Error('Cette fonction nécessite l\'application Juridix (hors navigateur de développement).'));
  return f(cmd, args);
}

export function isDesktopApp(): boolean {
  return !!(window as any).__TAURI__?.core?.invoke;
}

/** `localStorage` sérialisé, sans les éventuels secrets de repli (le backend les retire aussi). */
export function collectLocalData(): string {
  const out: Record<string, string> = {};
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (!k || k.startsWith('juridix_secret_')) continue;
    out[k] = localStorage.getItem(k) ?? '';
  }
  return JSON.stringify(out);
}

function isoDate(d = new Date()): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Demande l'emplacement (dialogue d'enregistrement) puis écrit l'archive. `null` = annulé. */
export async function exportUserData(password?: string): Promise<ExportReport | null> {
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) throw new Error('Cette fonction nécessite l\'application Juridix.');
  const dest = await internals.invoke('plugin:dialog|save', {
    payload: {
      defaultPath: `juridix-export-${isoDate()}.zip`,
      filters: [{ name: 'Archive ZIP', extensions: ['zip'] }],
    },
  }) as string | null;
  if (!dest) return null;
  return invoke<ExportReport>('export_user_data', {
    dest,
    frontendData: collectLocalData(),
    password: password?.trim() ? password : null,
  });
}

export function getBackupStatus(): Promise<BackupStatus> {
  return invoke<BackupStatus>('backup_status');
}

export function deleteBackup(): Promise<void> {
  return invoke<void>('delete_backup');
}

/**
 * Efface TOUT : base, sauvegarde, prompts, trousseau (backend), puis `localStorage` (ici), puis redémarre.
 * L'effacement du WebView n'a lieu qu'après la réussite du backend : en cas d'échec, rien n'est perdu côté interface.
 */
export async function wipeEverything(confirmation: string): Promise<WipeReport> {
  const report = await invoke<WipeReport>('wipe_user_data', { confirmation });
  // Verrou : jusqu'au redémarrage, plus aucun code de l'interface ne doit réécrire de données.
  Storage.prototype.setItem = function () { /* verrouillé après l'effacement */ };
  try { localStorage.clear(); sessionStorage.clear(); } catch { /* sans effet */ }
  // Redémarrage : le second temps de l'effacement (fichier de la base) s'exécute au lancement.
  invoke<void>('restart_app').catch(() => { /* géré par l'appelant : consigne de relance manuelle */ });
  return report;
}
