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

// Serveurs MCP officiels (Légifrance, Judilibre) : import depuis la release du
// pack, état, activation. Mêmes principes que le panneau MCP de Demeter, limité
// aux deux serveurs que Juridix gère. Les serveurs sont lancés par l'application
// en STDIO ; ici on les importe et on les active (`stdio:<id>` dans la liste des
// serveurs MCP des réglages).
import React, { useCallback, useEffect, useState } from 'react';
import { API_BASE } from '../constants';
import {
  formatTokens, sumTokens, toolsForMode, toolsLabel,
  type ToolCatalog, type ToolSelection,
} from '../utils/officialMcp';

export interface OfficialServerStatus {
  id: string;
  name: string;
  description: string;
  installed: boolean;
  available: boolean;
  running: boolean;
  needs_credentials: boolean;
  selection: ToolSelection;
  /** Outils offerts par le serveur ; `null` tant qu'il n'a jamais été interrogé. */
  tools_total: number | null;
  /** Outils exposés au LLM avec la sélection en vigueur. */
  tools_exposed: number | null;
}

interface ServersResponse {
  platform: string;
  pack_version: string | null;
  servers: OfficialServerStatus[];
}

interface UpdateResponse {
  installed: string | null;
  latest: string | null;
  update_available: boolean;
  error: string | null;
  /** Une clé de signature est compilée dans l'application : la signature des releases est obligatoire. */
  signature_enforced: boolean;
}

interface InstallReport {
  version: string;
  servers: { id: string; installed: boolean; error: string | null }[];
  /** `verified` : checksums.txt signé ; `not_configured` : seul le SHA-256 de la release a été contrôlé. */
  signature: 'verified' | 'not_configured';
}

const LABELS: Record<string, string> = {
  'mcp-legifrance': 'Légifrance',
  'mcp-judilibre': 'Judilibre',
};
const label = (s: { id: string; name?: string }) => LABELS[s.id] ?? s.name ?? s.id;

/** Valeur stockée dans `mcp_servers` pour un serveur géré. */
export const officialUrl = (id: string) => `stdio:${id}`;

interface Props {
  /** URLs actuellement présentes dans la liste des serveurs MCP du formulaire. */
  enabledUrls: string[];
  /** Ajoute (`on`) ou retire le serveur de la liste. */
  onToggle: (url: string, alias: string, on: boolean) => void;
}

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(`${API_BASE}${path}`);
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(body?.error || `Erreur ${r.status}`);
  return body as T;
}

/** Éditeur des outils exposés au LLM pour un serveur : Essentiels / Tous / Personnalisé. */
function ToolsEditor({ server, onSaved }: { server: OfficialServerStatus; onSaved: () => void }) {
  const [cat, setCat] = useState<ToolCatalog | null>(null);
  const [mode, setMode] = useState<ToolSelection['mode']>('essential');
  const [custom, setCustom] = useState<string[]>([]);
  const [filter, setFilter] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const load = useCallback(async () => {
    try {
      const c = await getJson<ToolCatalog>(`/api/mcp/stdio/tools?id=${encodeURIComponent(server.id)}`);
      setCat(c);
      setMode(c.selection.mode);
      setCustom(c.selection.mode === 'custom' ? c.selection.tools : c.tools.filter(t => t.essential).map(t => t.name));
      setErr(null);
    } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
  }, [server.id]);
  useEffect(() => { load(); }, [load]);

  const save = async () => {
    setSaving(true); setErr(null); setSaved(false);
    try {
      const body = mode === 'custom' ? { id: server.id, mode, tools: custom } : { id: server.id, mode };
      const r = await fetch(`${API_BASE}/api/mcp/stdio/tools`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      const data = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(data?.error || `Erreur ${r.status}`);
      setSaved(true);
      await load();
      onSaved();
    } catch (e) { setErr(e instanceof Error ? e.message : String(e)); }
    finally { setSaving(false); }
  };

  if (!cat) {
    return <div className="omcp__tools-editor">{err ? <div className="omcp__error" role="alert">{err}</div> : <span className="omcp__muted">Lecture du catalogue…</span>}</div>;
  }

  const q = filter.trim().toLowerCase();
  const visible = cat.tools.filter(t => !q || t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q));
  const essential = toolsForMode(cat, 'essential', []);
  const modeOption = (m: ToolSelection['mode'], title: string, tools: { length: number }, tokens: number, note?: string) => (
    <label className="omcp__mode">
      <input type="radio" name={`mode-${server.id}`} checked={mode === m} onChange={() => { setMode(m); setSaved(false); }} />
      <span><strong>{title}</strong> — {tools.length} outil{tools.length > 1 ? 's' : ''}, {formatTokens(tokens)} par requête{note ? ` ${note}` : ''}</span>
    </label>
  );
  const customTools = toolsForMode(cat, 'custom', custom);

  return (
    <div className="omcp__tools-editor" data-testid={`tools-${server.id}`}>
      <div className="omcp__muted">
        Chaque outil exposé ajoute sa définition à <em>chaque</em> requête de chat (coût en jetons) ; un catalogue trop large peut aussi nuire au choix de l'outil par le modèle.
      </div>
      {modeOption('essential', 'Essentiels', essential, sumTokens(essential), '(recommandé)')}
      {modeOption('all', 'Tous les outils', cat.tools, sumTokens(cat.tools))}
      {modeOption('custom', 'Personnalisé', customTools, sumTokens(customTools))}

      {mode === 'custom' && (
        <div className="omcp__custom">
          <div className="omcp__custom-bar">
            <input type="search" placeholder="Filtrer les outils…" value={filter} onChange={e => setFilter(e.target.value)} aria-label="Filtrer les outils" />
            <button type="button" className="omcp__link" onClick={() => setCustom(cat.tools.map(t => t.name))}>Tout cocher</button>
            <button type="button" className="omcp__link" onClick={() => setCustom([])}>Tout décocher</button>
            <button type="button" className="omcp__link" onClick={() => setCustom(essential.map(t => t.name))}>Essentiels</button>
          </div>
          <ul className="omcp__tool-list">
            {visible.map(t => (
              <li key={t.name}>
                <label title={t.description}>
                  <input type="checkbox" checked={custom.includes(t.name)}
                    onChange={e => { setSaved(false); setCustom(c => e.target.checked ? [...c, t.name] : c.filter(n => n !== t.name)); }} />
                  <code>{t.name}</code>
                  <span className="omcp__tool-tokens">{formatTokens(t.tokens)}</span>
                </label>
              </li>
            ))}
            {visible.length === 0 && <li className="omcp__muted">Aucun outil ne correspond.</li>}
          </ul>
        </div>
      )}

      {err && <div className="omcp__error" role="alert">{err}</div>}
      <div className="omcp__tools-actions">
        <button className="omcp__btn" onClick={save} disabled={saving || (mode === 'custom' && custom.length === 0)}>
          {saving ? 'Enregistrement…' : 'Appliquer'}
        </button>
        {saved && <span className="omcp__ok">Enregistré.</span>}
        {mode === 'custom' && custom.length === 0 && <span className="omcp__muted">Cochez au moins un outil.</span>}
      </div>
    </div>
  );
}

export function OfficialMcpPanel({ enabledUrls, onToggle }: Props) {
  const [data, setData] = useState<ServersResponse | null>(null);
  const [update, setUpdate] = useState<UpdateResponse | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<InstallReport | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);   // id du serveur dont l'éditeur d'outils est ouvert

  const refresh = useCallback(async () => {
    try {
      setData(await getJson<ServersResponse>('/api/mcp/stdio/servers'));
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const checkUpdate = useCallback(async () => {
    try { setUpdate(await getJson<UpdateResponse>('/api/mcp/stdio/pack-update')); }
    catch { /* hors ligne : on garde l'état précédent */ }
  }, []);

  useEffect(() => { refresh(); checkUpdate(); }, [refresh, checkUpdate]);

  // Le nombre réel d'outils n'est connu qu'une fois le serveur interrogé : on le relève en
  // arrière-plan pour afficher « 6 sur 62 » plutôt qu'un chiffre trompeur.
  const [probed, setProbed] = useState<string[]>([]);
  useEffect(() => {
    const todo = (data?.servers ?? []).filter(
      s => s.installed && s.available && !s.needs_credentials && s.tools_total == null && !probed.includes(s.id),
    );
    if (todo.length === 0) return;
    setProbed(p => [...p, ...todo.map(s => s.id)]);   // une seule tentative par serveur
    Promise.all(todo.map(s => getJson(`/api/mcp/stdio/tools?id=${encodeURIComponent(s.id)}`).catch(() => null))).then(() => refresh());
  }, [data, probed, refresh]);

  const install = async () => {
    setBusy(true);
    setInstallError(null);
    setReport(null);
    try {
      const r = await fetch(`${API_BASE}/api/mcp/stdio/pack-update`, { method: 'POST' });
      const body = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(body?.error || `Erreur ${r.status}`);
      setReport(body as InstallReport);
      await Promise.all([refresh(), checkUpdate()]);
    } catch (e) {
      setInstallError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const servers = data?.servers ?? [];
  const anyMissing = servers.some(s => !s.installed);
  const showImport = anyMissing || !!update?.update_available;
  const importLabel = anyMissing
    ? 'Importer Légifrance et Judilibre'
    : `Mettre à jour (v${update?.latest})`;

  const chip = (s: OfficialServerStatus): { text: string; kind: string } => {
    if (!s.installed) return { text: 'Non installé', kind: 'off' };
    if (!s.available) return { text: `Indisponible sur ${data?.platform ?? 'cette plateforme'}`, kind: 'err' };
    if (s.needs_credentials) return { text: 'Identifiants PISTE manquants', kind: 'warn' };
    return s.running ? { text: 'Démarré', kind: 'ok' } : { text: 'Prêt', kind: 'ok' };
  };

  return (
    <div className="omcp" data-testid="official-mcp">
      <div className="omcp__head">
        <span className="omcp__title">Serveurs officiels</span>
        {data?.pack_version && <span className="omcp__version">pack v{data.pack_version}</span>}
        {update && (
          <span
            className={update.signature_enforced ? 'omcp__sig omcp__sig--ok' : 'omcp__sig omcp__sig--warn'}
            title={update.signature_enforced
              ? 'Les releases du pack ne sont installées que si leur liste d\'empreintes porte une signature valide de Juridix.'
              : 'Aucune clé de signature n\'est compilée dans cette version : seul le SHA-256 fourni par la release est contrôlé, ce qui ne protège pas d\'une release compromise.'}
          >
            {update.signature_enforced ? '🔏 signature exigée' : '⚠ signature non configurée'}
          </span>
        )}
      </div>
      <div className="omcp__sub">
        Légifrance et Judilibre, lancés automatiquement par l'application. Les identifiants PISTE sont ceux des réglages juridiques.
      </div>

      {loadError && <div className="omcp__error" role="alert">{loadError}</div>}

      {servers.map(s => {
        const url = officialUrl(s.id);
        const enabled = enabledUrls.includes(url);
        const c = chip(s);
        const usable = s.installed && s.available;
        const ready = usable && !s.needs_credentials;
        return (
          <div key={s.id} className="omcp__server">
            <label className={`omcp__row ${usable ? '' : 'omcp__row--disabled'}`}>
              <input
                type="checkbox"
                checked={enabled}
                disabled={!usable}
                onChange={e => onToggle(url, label(s), e.target.checked)}
                aria-label={`Activer ${label(s)}`}
              />
              <span className="omcp__name">
                {label(s)}
                <span className="omcp__tools"> · {toolsLabel(s)}</span>
              </span>
              <span className={`omcp__chip omcp__chip--${c.kind}`}>{c.text}</span>
            </label>
            <button
              type="button"
              className="omcp__link omcp__tools-toggle"
              disabled={!ready}
              title={ready ? 'Choisir les outils exposés au LLM' : 'Disponible une fois le serveur installé et les identifiants PISTE renseignés'}
              aria-expanded={editing === s.id}
              onClick={() => setEditing(cur => (cur === s.id ? null : s.id))}
            >
              {editing === s.id ? 'Masquer les outils' : 'Choisir les outils…'}
            </button>
            {editing === s.id && ready && <ToolsEditor server={s} onSaved={refresh} />}
          </div>
        );
      })}

      {showImport && (
        <button className="omcp__btn" onClick={install} disabled={busy}>
          {busy ? 'Téléchargement…' : importLabel}
        </button>
      )}
      {update?.error && !showImport && servers.some(s => s.installed) && (
        <div className="omcp__muted" title={update.error}>Mises à jour non vérifiables (hors ligne ?)</div>
      )}
      {!showImport && update?.latest && !update.update_available && servers.length > 0 && (
        <div className="omcp__muted">À jour.</div>
      )}

      {installError && <div className="omcp__error" role="alert">{installError}</div>}
      {report?.servers.filter(s => !s.installed).map(s => (
        <div key={s.id} className="omcp__error" role="alert">
          {label(s)} : {s.error}
        </div>
      ))}
      {report && report.servers.every(s => s.installed) && (
        <div className="omcp__ok">
          Version {report.version} installée
          {report.signature === 'verified' ? ' (signature vérifiée)' : ' (signature non vérifiée : aucune clé configurée)'}.
          {' '}Cochez un serveur puis enregistrez pour l'activer.
        </div>
      )}
    </div>
  );
}
