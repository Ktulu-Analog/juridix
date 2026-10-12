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
  WIPE_WORD, deleteBackup, exportUserData, getBackupStatus, isDesktopApp, wipeEverything,
  type BackupStatus,
} from '../utils/rgpd';

type Msg = { kind: 'ok' | 'error'; text: string } | null;

function formatSize(n?: number): string {
  if (n == null) return '';
  if (n < 1024) return `${n} o`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} Ko`;
  return `${(n / (1024 * 1024)).toFixed(1)} Mo`;
}

/** Données et confidentialité : export, sauvegarde en clair, effacement complet (RGPD). */
export function DataPrivacyModal({ onClose }: { onClose: () => void }) {
  const desktop = isDesktopApp();
  const [backup, setBackup] = useState<BackupStatus | null>(null);
  const [confirmBackup, setConfirmBackup] = useState(false);
  const [pwd, setPwd] = useState('');
  const [pwd2, setPwd2] = useState('');
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState<'export' | 'backup' | 'wipe' | null>(null);
  const [msg, setMsg] = useState<Msg>(null);
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    if (!desktop) return;
    getBackupStatus().then(setBackup).catch(() => setBackup(null));
  }, [desktop]);

  const doExport = async () => {
    if (pwd !== pwd2) { setMsg({ kind: 'error', text: 'Les deux mots de passe ne correspondent pas.' }); return; }
    setBusy('export'); setMsg(null);
    try {
      const r = await exportUserData(pwd || undefined);
      if (r) {
        setMsg({
          kind: 'ok',
          text: `Archive enregistrée (${r.conversations} conversation${r.conversations > 1 ? 's' : ''}, ${r.fichiers} fichiers`
            + `${r.chiffree ? ', chiffrée AES-256' : ', non chiffrée : conservez-la en lieu sûr'}) : ${r.path}`,
        });
      }
    } catch (e) {
      setMsg({ kind: 'error', text: `Export impossible : ${e instanceof Error ? e.message : String(e)}` });
    } finally { setBusy(null); }
  };

  const doDeleteBackup = async () => {
    setBusy('backup'); setMsg(null);
    try {
      await deleteBackup();
      setBackup(await getBackupStatus());
      setConfirmBackup(false);
      setMsg({ kind: 'ok', text: 'Sauvegarde en clair supprimée.' });
    } catch (e) {
      setMsg({ kind: 'error', text: `Suppression impossible : ${e instanceof Error ? e.message : String(e)}` });
    } finally { setBusy(null); }
  };

  const doWipe = async () => {
    setBusy('wipe'); setMsg(null);
    try {
      const r = await wipeEverything(typed);
      setRestarting(true);
      if (r.avertissements.length > 0) {
        setMsg({ kind: 'error', text: `Effacement effectué, avec réserves : ${r.avertissements.join(' · ')}` });
      }
    } catch (e) {
      setMsg({ kind: 'error', text: `Effacement impossible : ${e instanceof Error ? e.message : String(e)}` });
      setBusy(null);
    }
  };

  if (restarting) {
    return (
      <div className="modal-overlay">
        <div className="modal data-modal">
          <div className="modal-body">
            <h3>Données effacées</h3>
            <p>L'application redémarre pour terminer l'effacement. Si elle ne se relance pas d'elle-même
              (par exemple en mode développement), fermez-la puis relancez-la : l'effacement s'achèvera au démarrage.</p>
            {msg && <div className={`data-msg data-msg--${msg.kind}`} role="alert">{msg.text}</div>}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal-overlay">
      <div className="modal data-modal">
        <div className="modal-header"><h3>🔒 Données et confidentialité</h3></div>
        <div className="modal-body">
          {!desktop && <div className="data-msg data-msg--error">Ces fonctions ne sont disponibles que dans l'application Juridix.</div>}

          <section className="data-section">
            <h4>Exporter mes données</h4>
            <p>
              Archive .zip : conversations (JSON et version lisible), veilles et résultats, dossiers, favoris,
              annotations, historique et réglages d'interface. Vos identifiants (jeton, clés) ne sont pas exportés ;
              les documents de vos collections Qdrant distantes non plus.
            </p>
            <div className="data-row">
              <input type="password" placeholder="Mot de passe de l'archive (facultatif)" value={pwd}
                     autoComplete="new-password" onChange={e => setPwd(e.target.value)} />
              <input type="password" placeholder="Confirmer le mot de passe" value={pwd2}
                     autoComplete="new-password" onChange={e => setPwd2(e.target.value)} disabled={!pwd} />
            </div>
            <p className="data-hint">
              Sans mot de passe, l'archive contient vos données en clair. Avec un mot de passe, elle est chiffrée
              (AES-256) ; l'explorateur de Windows ne sait pas l'ouvrir : utilisez 7-Zip, Keka ou équivalent.
            </p>
            <button className="btn-primary" onClick={doExport} disabled={!desktop || busy !== null}>
              {busy === 'export' ? 'Export en cours…' : 'Exporter mes données…'}
            </button>
          </section>

          {backup?.exists && (
            <section className="data-section data-section--warn">
              <h4>⚠️ Sauvegarde en clair de l'ancienne base</h4>
              <p>
                La migration vers le stockage chiffré a laissé une copie <strong>non chiffrée</strong> de vos anciennes
                conversations sur ce disque ({formatSize(backup.sizeBytes)}, <code>{backup.path}</code>).
                Une fois la migration vérifiée, supprimez-la.
              </p>
              {!confirmBackup ? (
                <button className="btn-ghost" onClick={() => setConfirmBackup(true)} disabled={busy !== null}>
                  Supprimer la sauvegarde en clair…
                </button>
              ) : (
                <div className="data-row">
                  <span>Suppression définitive : confirmer ?</span>
                  <button className="btn-danger" onClick={doDeleteBackup} disabled={busy !== null}>
                    {busy === 'backup' ? 'Suppression…' : 'Oui, supprimer'}
                  </button>
                  <button className="btn-ghost" onClick={() => setConfirmBackup(false)} disabled={busy !== null}>Annuler</button>
                </div>
              )}
            </section>
          )}

          <section className="data-section data-section--danger">
            <h4>Tout effacer</h4>
            <p>
              Supprime définitivement, sur ce poste : conversations, veilles, dossiers, favoris, annotations,
              historique, réglages, identifiants du trousseau, clé de chiffrement de la base et sauvegarde en clair.
              L'application redémarre ensuite. Cette action est irréversible : pensez à exporter vos données d'abord.
            </p>
            <p className="data-hint">
              Non concernés : vos collections Qdrant distantes et les serveurs MCP installés. Sur un disque SSD,
              l'écrasement des fichiers ne garantit pas un effacement physique : pour une assurance complète,
              activez le chiffrement intégral du disque.
            </p>
            <div className="data-row">
              <input value={typed} placeholder={`Tapez ${WIPE_WORD} pour confirmer`}
                     onChange={e => setTyped(e.target.value)} aria-label={`Tapez ${WIPE_WORD} pour confirmer`} />
              <button className="btn-danger" onClick={doWipe} disabled={!desktop || typed !== WIPE_WORD || busy !== null}>
                {busy === 'wipe' ? 'Effacement…' : 'Tout effacer'}
              </button>
            </div>
          </section>

          {msg && <div className={`data-msg data-msg--${msg.kind}`} role="alert">{msg.text}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn-ghost" onClick={onClose} disabled={busy === 'wipe'}>Fermer</button>
        </div>
      </div>
    </div>
  );
}
