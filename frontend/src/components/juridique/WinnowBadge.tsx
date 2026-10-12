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

// ============================================================================
// Espace "Recherche juridique" — Pré-filtre local Winnow
// ============================================================================
// Deux éléments d'interface, tous deux exportés par ce fichier :
//
//   - `WinnowIndicator` : pastille toujours visible à côté des modes d'analyse
//     de dossier, qui signale si le pré-filtre est actif et ce qu'il a fait
//     lors de la dernière analyse. Cliquable → ouvre `WinnowModal`.
//   - `WinnowModal`     : fenêtre dédiée détaillant la sortie de Winnow (état
//     d'Ollaya, statistiques, liste intégrale des passages écartés).
// ============================================================================

import { useEffect, useState } from 'react';
import { API_BASE } from '../../constants';
import type { WinnowReport, WinnowStatus } from './types';

// ── État affiché par la pastille ─────────────────────────────────────────────

type PillState =
  | { kind: 'off' }
  | { kind: 'checking' }
  | { kind: 'unavailable'; hint: string | null }
  | { kind: 'ready' }
  | { kind: 'fallback'; reason: string }
  | { kind: 'applied'; dropped: number; total: number; savedPct: number };

function computePillState(
  enabled: boolean,
  status: WinnowStatus | null,
  statusLoading: boolean,
  report: WinnowReport | null,
): PillState {
  // Le dernier rapport prime : il décrit ce qui vient réellement de se passer.
  if (report) {
    if (!report.enabled) return { kind: 'off' };
    if (report.applied) {
      const savedPct = report.chars_before > 0
        ? Math.round((1 - report.chars_after / report.chars_before) * 100)
        : 0;
      return { kind: 'applied', dropped: report.dropped, total: report.total_passages, savedPct };
    }
    return { kind: 'fallback', reason: report.fallback || 'non appliqué' };
  }
  if (!enabled) return { kind: 'off' };
  if (statusLoading) return { kind: 'checking' };
  if (!status?.reachable || !status.model_installed) return { kind: 'unavailable', hint: status?.hint ?? null };
  return { kind: 'ready' };
}

const PILL_LABEL: Record<PillState['kind'], string> = {
  off: '🌾 Winnow désactivé',
  checking: '🌾 Winnow…',
  unavailable: '🌾 Winnow inactif',
  ready: '🌾 Winnow prêt',
  fallback: '🌾 Winnow non appliqué',
  applied: '🌾 Winnow',
};

// ── Pastille ──────────────────────────────────────────────────────────────────

export function WinnowIndicator({ enabled, report }: { enabled: boolean; report: WinnowReport | null }) {
  const [status, setStatus] = useState<WinnowStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);

  // Statut d'Ollaya : tant qu'aucune analyse n'a produit de rapport, et
  // seulement si le filtre est activé (pas de sonde réseau inutile sinon).
  useEffect(() => {
    if (report || !enabled) { setStatus(null); return; }
    let cancelled = false;
    setStatusLoading(true);
    fetch(`${API_BASE}/api/juridique/winnow-status?enabled=true`)
      .then((r) => (r.ok ? r.json() : null))
      .then((s: WinnowStatus | null) => { if (!cancelled) setStatus(s); })
      .catch(() => { /* silencieux : fonction optionnelle, échec ouvert */ })
      .finally(() => { if (!cancelled) setStatusLoading(false); });
    return () => { cancelled = true; };
  }, [report, enabled]);

  const state = computePillState(enabled, status, statusLoading, report);
  // Rien à montrer dans la modale tant que le filtre n'a jamais tourné et
  // qu'aucun problème n'est détecté : éviter une pastille cliquable vide.
  const clickable = state.kind !== 'off' && state.kind !== 'checking';

  return (
    <>
      <button
        type="button"
        className={`jur-winnow-pill jur-winnow-pill--${state.kind}`}
        onClick={() => clickable && setModalOpen(true)}
        disabled={!clickable}
        title={clickable ? 'Voir le détail du pré-filtre local' : undefined}
      >
        {state.kind === 'applied'
          ? <>🌾 Winnow · <strong>{state.dropped}</strong>/{state.total} écarté{state.dropped > 1 ? 's' : ''}{state.savedPct > 0 && <> · −{state.savedPct} %</>}</>
          : PILL_LABEL[state.kind]}
      </button>
      {modalOpen && (
        <WinnowModal report={report} status={status} onClose={() => setModalOpen(false)} />
      )}
    </>
  );
}

// ── Modale dédiée ─────────────────────────────────────────────────────────────

function WinnowModal({
  report,
  status,
  onClose,
}: {
  report: WinnowReport | null;
  status: WinnowStatus | null;
  onClose: () => void;
}) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal jur-winnow-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <div className="modal-header-left">
            <span className="modal-icon">🌾</span>
            <span className="modal-title">Pré-filtre local Winnow</span>
          </div>
          <button className="icon-btn" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body jur-winnow-modal-body">
          {report ? <WinnowReportView report={report} /> : <WinnowStatusOnlyView status={status} />}
        </div>
        <div className="modal-footer">
          <button className="btn-ghost" onClick={onClose}>Fermer</button>
        </div>
      </div>
    </div>
  );
}

function WinnowStatusOnlyView({ status }: { status: WinnowStatus | null }) {
  if (!status) {
    return <p className="jur-winnow-modal-empty">État d'Ollaya en cours de vérification…</p>;
  }
  return (
    <>
      <p className="jur-winnow-modal-empty">
        Aucune analyse de dossier n'a encore été lancée. Voici l'état actuel du serveur local :
      </p>
      <table className="jur-winnow-table">
        <tbody>
          <tr><td>Serveur Ollaya</td><td>{status.reachable ? '✅ joignable' : '❌ injoignable'}{status.version ? ` (v${status.version})` : ''}</td></tr>
          <tr><td>Modèle</td><td>{status.model}</td></tr>
          <tr><td>Installé</td><td>{status.model_installed ? '✅ oui' : '❌ non'}</td></tr>
          <tr><td>Chargé en mémoire</td><td>{status.model_loaded ? '✅ oui' : '— pas encore'}</td></tr>
          {status.device && <tr><td>Périphérique</td><td>{status.device}</td></tr>}
        </tbody>
      </table>
      {status.hint && <div className="jur-winnow-note jur-winnow-note--warn">{status.hint}</div>}
      <p className="jur-winnow-modal-empty">
        Voir <code>docs/WINNOW.md</code> pour l'installation.
      </p>
    </>
  );
}

function WinnowReportView({ report }: { report: WinnowReport }) {
  const saved = report.chars_before > 0
    ? Math.round((1 - report.chars_after / report.chars_before) * 100)
    : 0;

  return (
    <>
      <table className="jur-winnow-table">
        <tbody>
          <tr><td>Modèle</td><td>{report.model}</td></tr>
          <tr><td>Passages</td><td>{report.total_passages} au total · {report.scored} évalués · {report.unscored} conservés sans évaluation</td></tr>
          <tr><td>Résultat</td><td><strong>{report.dropped}</strong> écarté{report.dropped > 1 ? 's' : ''} · {report.kept} conservé{report.kept > 1 ? 's' : ''}</td></tr>
          <tr><td>Texte envoyé</td><td>{report.chars_before.toLocaleString()} → {report.chars_after.toLocaleString()} caractères{saved > 0 && ` (−${saved} %)`}</td></tr>
          <tr><td>Durée</td><td>{report.elapsed_ms} ms</td></tr>
        </tbody>
      </table>

      {report.fallback && (
        <div className="jur-winnow-note jur-winnow-note--warn">
          Filtrage partiel ou non appliqué : {report.fallback}. Le texte concerné est parti intégralement.
        </div>
      )}

      {report.dropped_items.length === 0 ? (
        <p className="jur-winnow-modal-empty">Aucun passage écarté lors de cette analyse.</p>
      ) : (
        <>
          <div className="jur-winnow-modal-subtitle">
            Passages écartés — à vérifier avant de vous fier à l'absence d'une disposition
          </div>
          <ul className="jur-winnow-modal-list">
            {report.dropped_items.map((it, i) => (
              <li key={i}>
                <div className="jur-winnow-modal-list-head">
                  <span className="jur-tag jur-tag--xs">{it.reason} · {Math.round(it.p * 100)} %</span>
                  <em>{it.doc_titre}</em>{it.heading ? ` › ${it.heading}` : ''}
                </div>
                <div className="jur-winnow-preview">{it.preview}…</div>
              </li>
            ))}
          </ul>
          {report.dropped > report.dropped_items.length && (
            <p className="jur-winnow-modal-empty">
              + {report.dropped - report.dropped_items.length} autre(s), non détaillé(s) au-delà de 200 entrées.
            </p>
          )}
        </>
      )}
    </>
  );
}
