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

// Autorisation des serveurs MCP (R-02) — côté frontend.
//
// Principe : une autorisation GLOBALE par serveur (tous ses outils), demandée une fois puis
// renouvelée seulement si le serveur modifie ses outils — jamais une confirmation par appel.
// L'application de la règle se fait côté backend (src-tauri/src/mcp_grants.rs) : ici on ne fait
// qu'afficher les outils, recueillir l'accord et le transmettre.

import { API_BASE } from '../constants';

export type GateStatus = 'trusted' | 'granted' | 'new' | 'changed' | 'unavailable';

export interface ToolSummary { name: string; description: string; }

export interface McpGrant { server: string; fingerprint: string; tools: string[]; granted_at: string; }

export interface McpInspection {
  server: string;
  status: GateStatus;
  reachable: boolean;
  /** Empreinte des outils affichés : renvoyée telle quelle pour autoriser exactement ce qui a été vu. */
  fingerprint: string;
  tools: ToolSummary[];
  granted_at: string | null;
}

/** Serveur écarté par le backend pour un message (nouveau, ou outils modifiés depuis l'autorisation). */
export interface McpBlocked { server: string; status: GateStatus; fingerprint: string; tools: ToolSummary[]; }

/** Les serveurs officiels `stdio:` sont fiables par construction : aucune autorisation à demander. */
export function needsGrant(server: string): boolean {
  return !/^stdio:/i.test(server.trim());
}

/** Serveur sur cette machine (localhost, 127.0.0.1, ::1). */
export function isLocalServer(server: string): boolean {
  try {
    const h = new URL(server).hostname.toLowerCase();
    return h === 'localhost' || h === '127.0.0.1' || h === '[::1]' || h === '::1' || h.endsWith('.localhost');
  } catch {
    return false;
  }
}

export function serverHost(server: string): string {
  try { return new URL(server).host; } catch { return server; }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((data as { error?: string }).error || `Erreur ${res.status}`);
  return data as T;
}

export async function listGrants(): Promise<McpGrant[]> {
  return (await call<{ grants: McpGrant[] }>('/api/mcp/grants')).grants;
}

export function inspectServer(server: string): Promise<McpInspection> {
  return call<McpInspection>('/api/mcp/grants/inspect', { method: 'POST', body: JSON.stringify({ server }) });
}

export async function grantServer(server: string, fingerprint: string): Promise<void> {
  await call('/api/mcp/grants', { method: 'PUT', body: JSON.stringify({ server, fingerprint }) });
}

export async function revokeServer(server: string): Promise<void> {
  await call('/api/mcp/grants/revoke', { method: 'POST', body: JSON.stringify({ server }) });
}

// ── Signalement « serveur non autorisé » (événement SSE `mcp_blocked` → application) ─────────────

const BLOCKED_EVENT = 'juridix:mcp-blocked';

export function notifyBlocked(servers: McpBlocked[]): void {
  window.dispatchEvent(new CustomEvent(BLOCKED_EVENT, { detail: servers }));
}

export function onMcpBlocked(cb: (servers: McpBlocked[]) => void): () => void {
  const handler = (e: Event) => cb(((e as CustomEvent).detail ?? []) as McpBlocked[]);
  window.addEventListener(BLOCKED_EVENT, handler);
  return () => window.removeEventListener(BLOCKED_EVENT, handler);
}
