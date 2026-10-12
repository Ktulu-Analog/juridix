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

import React, { useEffect, useState } from 'react';
import {
  grantServer, inspectServer, isLocalServer, revokeServer, serverHost,
  type McpInspection,
} from '../utils/mcpGrants';

interface Props {
  /** URL du serveur MCP (identifiant côté backend). */
  server: string;
  alias?: string;
  /** `changed` = true si une autorisation a été accordée ou retirée (pour rafraîchir l'appelant). */
  onClose: (changed: boolean) => void;
}

/**
 * Autorisation d'un serveur MCP : l'utilisateur voit TOUS les outils du serveur et l'autorise d'un
 * seul geste ; ensuite le modèle les appelle sans rien demander. L'autorisation tombe si le serveur
 * modifie ses outils.
 */
export function McpAuthDialog({ server, alias, onClose }: Props) {
  const [info, setInfo] = useState<McpInspection | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    inspectServer(server)
      .then(i => { if (alive) setInfo(i); })
      .catch(e => { if (alive) setError(e instanceof Error ? e.message : String(e)); });
    return () => { alive = false; };
  }, [server]);

  const label = alias || serverHost(server);

  const authorize = async () => {
    if (!info) return;
    setBusy(true); setError(null);
    try { await grantServer(server, info.fingerprint); onClose(true); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };

  const revoke = async () => {
    setBusy(true); setError(null);
    try { await revokeServer(server); onClose(true); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); setBusy(false); }
  };

  return (
    <div className="modal-overlay" style={{ zIndex: 950 }}>
      <div className="modal mcp-auth" role="dialog" aria-label={`Autoriser le serveur MCP ${label}`}>
        <div className="modal-header"><h3>🔐 Serveur MCP « {label} »</h3></div>
        <div className="modal-body">
          {!info && !error && <p role="status">Examen des outils du serveur…</p>}
          {error && <div className="data-msg data-msg--error" role="alert">{error}</div>}

          {info && !info.reachable && (
            <div className="data-msg data-msg--error" role="alert">
              Ce serveur est injoignable : impossible d'examiner ses outils pour l'instant.
            </div>
          )}

          {info && info.reachable && (
            <>
              {info.status === 'changed' && (
                <div className="mcp-warn" role="alert">
                  <div className="mcp-warn__title">⚠️ Les outils de ce serveur ont changé</div>
                  Ils ne sont plus ceux que vous aviez autorisés : le modèle ne les utilise pas tant que vous
                  ne les avez pas réexaminés. Vérifiez-les avant d'autoriser à nouveau.
                </div>
              )}
              {info.status === 'granted' && (
                <p className="data-hint">✔ Autorisé{info.granted_at ? ` le ${new Date(info.granted_at).toLocaleDateString('fr-FR')}` : ''} — ces outils sont ceux que vous avez examinés.</p>
              )}
              {!isLocalServer(server) && info.status !== 'granted' && (
                <p className="data-hint">
                  Serveur distant ({serverHost(server)}) : il recevra le contenu des requêtes que le modèle lui
                  adresse. N'autorisez qu'un serveur en qui vous avez confiance.
                </p>
              )}
              <p>
                {info.tools.length} outil{info.tools.length > 1 ? 's' : ''}. En autorisant ce serveur, le modèle
                pourra les appeler <strong>sans vous demander à chaque fois</strong>. Si le serveur modifie ses
                outils, l'autorisation sera à renouveler.
              </p>
              <ul className="mcp-auth__tools">
                {info.tools.map(t => (
                  <li key={t.name}>
                    <code>{t.name}</code>
                    {t.description && <span> — {t.description}</span>}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
        <div className="modal-footer">
          {info?.status === 'granted' && (
            <button className="btn-danger" onClick={revoke} disabled={busy}>Retirer l'autorisation</button>
          )}
          <button className="btn-ghost" onClick={() => onClose(false)} disabled={busy}>
            {info?.status === 'granted' ? 'Fermer' : 'Ne pas autoriser'}
          </button>
          {info && info.reachable && info.status !== 'granted' && info.tools.length > 0 && (
            <button className="btn-primary" onClick={authorize} disabled={busy}>
              Autoriser les {info.tools.length} outil{info.tools.length > 1 ? 's' : ''}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
