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
// Juridix — Réglage « Droit applicable au… » (date des faits)
// ============================================================================
// La date n'est appliquée qu'à la validation explicite (bouton « Appliquer » ou
// Entrée), jamais à chaque changement du champ : un sélecteur de date émet un
// évènement à chaque clic sur une année ou un mois, et chaque application relance
// la vérification des citations de toute la conversation.
// ============================================================================
import { useEffect, useState, type ReactNode } from 'react';
import {
  ANNEE_MAX, ANNEE_MIN, formatDroitADate, isValidIsoDate, setDroitADate, useDroitADate,
} from '../utils/droitADate';

const HELP =
  "Applique le droit tel qu'il était en vigueur à la date des faits : le modèle en est informé, " +
  'les articles cités sont vérifiés dans leur version de cette date, et la recherche dans les codes ' +
  "renvoie les versions applicables. Non conservé d'une session à l'autre.";

const MIN = `${ANNEE_MIN}-01-01`;
const MAX = `${ANNEE_MAX}-12-31`;

/** Champ de date avec validation explicite. */
function DateEditor({ initial, onDone, autoFocus, extra }: {
  initial: string | null;
  /** Bouton supplémentaire, rangé avec « Appliquer » (ex. « Droit actuel »). */
  extra?: ReactNode;
  /** Appelé après « Appliquer » ou « Annuler ». Sans lui, pas de bouton « Annuler » (champ toujours affiché). */
  onDone?: () => void;
  autoFocus?: boolean;
}) {
  const [draft, setDraft] = useState(initial ?? '');
  // Si la date change ailleurs (puce vidée, autre champ), le brouillon suit.
  useEffect(() => { setDraft(initial ?? ''); }, [initial]);

  const valid = isValidIsoDate(draft);
  const apply = () => {
    if (!valid) return;
    setDroitADate(draft);
    onDone?.();
  };

  return (
    <>
      <input
        type="date"
        className="droit-date-input"
        aria-label="Droit applicable au (date des faits)"
        value={draft}
        min={MIN}
        max={MAX}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') { e.preventDefault(); apply(); }
          else if (e.key === 'Escape' && onDone) { e.preventDefault(); onDone(); }
        }}
        autoFocus={autoFocus}
      />
      <span className="droit-date-actions">
        <button
          type="button"
          className="droit-date-apply"
          disabled={!valid || draft === (initial ?? '')}
          onClick={apply}
        >
          Appliquer
        </button>
        {onDone && <button type="button" className="droit-date-clear" onClick={onDone}>Annuler</button>}
        {extra}
      </span>
    </>
  );
}

/**
 * `variant="chat"` : puce compacte au-dessus de la zone de saisie.
 * `variant="field"` : champ libellé (panneau de recherche).
 */
export function DroitADateControl({ variant = 'chat' }: { variant?: 'chat' | 'field' }) {
  const date = useDroitADate();
  const [editing, setEditing] = useState(false);

  if (variant === 'field') {
    return (
      <div className="jur-facet-body droit-date-field">
        <div className="droit-date-row">
          <DateEditor
            initial={date}
            extra={date && (
              <button type="button" className="droit-date-clear" onClick={() => setDroitADate(null)}>
                Droit actuel
              </button>
            )}
          />
        </div>
        <small className="droit-date-help">{HELP}</small>
      </div>
    );
  }

  if (editing) {
    return (
      <div className="droit-date-bar">
        <span>Droit applicable au</span>
        <DateEditor initial={date} onDone={() => setEditing(false)} autoFocus />
      </div>
    );
  }

  return (
    <div className="droit-date-bar">
      {date ? (
        <span className="droit-date-chip" title={HELP}>
          <button type="button" className="droit-date-chip-main" onClick={() => setEditing(true)}>
            ⚖️ Droit au {formatDroitADate(date)}
          </button>
          <button
            type="button"
            className="droit-date-chip-x"
            aria-label="Revenir au droit actuel"
            onClick={() => setDroitADate(null)}
          >✕</button>
        </span>
      ) : (
        <button type="button" className="droit-date-add" title={HELP} onClick={() => setEditing(true)}>
          ⚖️ Droit applicable à une date…
        </button>
      )}
    </div>
  );
}
