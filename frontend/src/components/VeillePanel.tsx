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

// Veille juridique : liste des veilles, résultats détectés, création/modification.
// Le backend (veille.rs) rejoue les recherches en tâche de fond ; ce panneau
// affiche ce qu'il a trouvé de nouveau et permet d'ouvrir chaque texte.
import React, { useCallback, useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ARTIFACT_MD_COMPONENTS } from './MarkdownComponents';
import { normalizeHeadings, normalizeLatex } from '../utils/text';
import { generateDocx } from '../utils/docx';
import { MAX_TEXTES, assemblerNote, chargerTextes, demanderPrompt, nomFichierNote } from '../utils/veilleSynthese';
import {
  CODES_VEILLE, FONDS_AVEC_NATURE, FOND_OPTIONS, FREQUENCES, JURIDICTIONS, NATURES,
  describeCriteres, formatTexteDate, relativeTime, summarizeRun, veilleApi,
  type Veille, type VeilleCriteres, type VeilleInput, type VeilleLimites, type VeilleResultat,
} from '../utils/veille';

const PAGE = 50;

const emptyCriteres = (): VeilleCriteres => ({
  source: 'legifrance', query: '', fond: 'LODA_DATE', natures: [], code: null, juridictions: [],
});

interface FormState { nom: string; criteres: VeilleCriteres; frequence: number; depuis: string; }

const toForm = (v: Veille): FormState => ({
  nom: v.nom, criteres: { ...v.criteres, natures: [...v.criteres.natures], juridictions: [...v.criteres.juridictions] },
  frequence: v.frequence_heures, depuis: v.depuis,
});

type SynthPhase = 'idle' | 'config' | 'chargement' | 'generation' | 'pret' | 'erreur';

interface SynthState {
  phase: SynthPhase;
  consigne: string;
  faits: number;
  total: number;
  note: string;
  nbAnalyses: number;
  nbEchecs: number;
  nbTronques: number;
  erreur: string | null;
}

const SYNTH_INIT: SynthState = {
  phase: 'idle', consigne: '', faits: 0, total: 0, note: '', nbAnalyses: 0, nbEchecs: 0, nbTronques: 0, erreur: null,
};

interface Props {
  onClose: () => void;
  /** Ouvre le résultat dans la visionneuse de documents. */
  onOpenResult: (r: VeilleResultat) => void;
  /** Appelé quand le nombre de non lus a pu changer (met à jour le badge global). */
  onChanged?: () => void;
  /**
   * Envoie le prompt de synthèse au chat et renvoie la réponse finale du modèle (null si
   * la génération a échoué ou été interrompue). Absent = la synthèse n'est pas proposée.
   */
  onSynthese?: (prompt: string) => Promise<string | null>;
  /** Préfiltre local Winnow activé dans les préférences. */
  winnowEnabled?: boolean;
}

export function VeillePanel({ onClose, onOpenResult, onChanged, onSynthese, winnowEnabled }: Props) {
  const [veilles, setVeilles] = useState<Veille[]>([]);
  const [limites, setLimites] = useState<VeilleLimites | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [resultats, setResultats] = useState<VeilleResultat[]>([]);
  const [totalRes, setTotalRes] = useState(0);
  const [onlyUnread, setOnlyUnread] = useState(false);
  const [form, setForm] = useState<FormState | null>(null);   // non null = création ou édition
  const [editingId, setEditingId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  // Sélection de textes pour la synthèse (identifiants de résultats) et état de la synthèse.
  const [choisis, setChoisis] = useState<Set<number>>(new Set());
  const [synth, setSynth] = useState<SynthState>(SYNTH_INIT);
  // Changer de veille : la sélection et la synthèse en cours ne s'appliquent plus.
  useEffect(() => { setChoisis(new Set()); setSynth(SYNTH_INIT); }, [selectedId]);

  const selected = veilles.find(v => v.id === selectedId) ?? null;

  const reload = useCallback(async (keepSelection = true) => {
    try {
      const r = await veilleApi.list();
      setVeilles(r.veilles);
      setLimites(r.limites);
      setSelectedId(cur => (keepSelection && cur && r.veilles.some(v => v.id === cur)) ? cur : (r.veilles[0]?.id ?? null));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoaded(true);
    }
  }, []);

  const loadResultats = useCallback(async (id: string, unread: boolean, offset = 0) => {
    try {
      const r = await veilleApi.resultats(id, { nonLus: unread, limit: PAGE, offset });
      setResultats(prev => offset === 0 ? r.resultats : [...prev, ...r.resultats]);
      setTotalRes(unread ? r.non_lus : r.total);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => { reload(false); }, [reload]);
  useEffect(() => {
    if (selectedId) loadResultats(selectedId, onlyUnread); else { setResultats([]); setTotalRes(0); }
  }, [selectedId, onlyUnread, loadResultats]);

  const refreshAll = async () => {
    await reload();
    if (selectedId) await loadResultats(selectedId, onlyUnread);
    onChanged?.();
  };

  // ── Actions ────────────────────────────────────────────────────────────────

  const startCreate = () => {
    setEditingId(null); setInfo(null); setError(null);
    setForm({ nom: '', criteres: emptyCriteres(), frequence: limites?.frequence_defaut_h ?? 24, depuis: '' });
  };
  const startEdit = (v: Veille) => { setEditingId(v.id); setInfo(null); setError(null); setForm(toForm(v)); };

  const submit = async () => {
    if (!form) return;
    setBusy(true); setError(null); setInfo(null);
    const c = form.criteres;
    const input: VeilleInput = {
      nom: form.nom,
      criteres: c,
      frequence_heures: form.frequence,
      ...(form.depuis ? { depuis: form.depuis } : {}),
    };
    try {
      let saved: Veille;
      if (editingId) {
        saved = await veilleApi.update(editingId, { ...input, active: veilles.find(v => v.id === editingId)?.active ?? true });
      } else {
        saved = await veilleApi.create(input);
      }
      setForm(null); setEditingId(null);
      setSelectedId(saved.id);
      // Première vérification immédiate : l'utilisateur voit tout de suite ce que la veille détecte.
      try {
        const run = await veilleApi.executer(saved.id);
        setInfo(summarizeRun(run));
      } catch (e) {
        setInfo(`Veille enregistrée. Première vérification impossible pour l'instant : ${e instanceof Error ? e.message : e}`);
      }
      await refreshAll();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const runNow = async () => {
    if (!selected) return;
    setBusy(true); setError(null); setInfo(null);
    try { setInfo(summarizeRun(await veilleApi.executer(selected.id))); await refreshAll(); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };

  const toggleActive = async (v: Veille) => {
    setError(null);
    try {
      await veilleApi.update(v.id, { nom: v.nom, criteres: v.criteres, active: !v.active, frequence_heures: v.frequence_heures, depuis: v.depuis });
      await refreshAll();
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };

  const remove = async (v: Veille) => {
    if (!window.confirm(`Supprimer la veille « ${v.nom} » et ses ${v.total} résultat(s) ?`)) return;
    try { await veilleApi.remove(v.id); setSelectedId(null); await reload(false); onChanged?.(); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };

  const markAllRead = async () => {
    if (!selected) return;
    try { await veilleApi.marquerLus(selected.id); await refreshAll(); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };

  const open = async (r: VeilleResultat) => {
    if (!r.vu) {
      setResultats(prev => prev.map(x => x.id === r.id ? { ...x, vu: true } : x));
      setVeilles(prev => prev.map(v => v.id === r.veille_id ? { ...v, non_lus: Math.max(0, v.non_lus - 1) } : v));
      veilleApi.marquerLus(r.veille_id, [r.id]).then(() => onChanged?.()).catch(() => { /* non bloquant */ });
    }
    onOpenResult(r);
  };

  // ── Synthèse de veille ─────────────────────────────────────────────────────

  const resultatsChoisis = resultats.filter(r => choisis.has(r.id));
  const nbChoisis = resultatsChoisis.length;
  const synthEnCours = synth.phase === 'chargement' || synth.phase === 'generation';
  const tousChoisis = resultats.length > 0
    && resultats.slice(0, MAX_TEXTES).every(r => choisis.has(r.id)) && nbChoisis === Math.min(MAX_TEXTES, resultats.length);

  const toggleChoix = (id: number) => setChoisis(prev => {
    const n = new Set(prev);
    if (n.has(id)) n.delete(id); else n.add(id);
    return n;
  });
  const toutChoisir = () => setChoisis(tousChoisis ? new Set() : new Set(resultats.slice(0, MAX_TEXTES).map(r => r.id)));

  const lancerSynthese = async () => {
    if (!selected || !onSynthese || nbChoisis === 0) return;
    const liste = resultatsChoisis;
    setSynth(s => ({ ...s, phase: 'chargement', faits: 0, total: liste.length, erreur: null }));
    try {
      const textes = await chargerTextes(liste, (faits, total) => setSynth(s => ({ ...s, faits, total })));
      const charges = textes.filter(t => !t.erreur);
      if (charges.length === 0) throw new Error('Aucun des textes sélectionnés n\'a pu être chargé.');
      const prompt = await demanderPrompt(selected.nom, synth.consigne, charges, !!winnowEnabled);
      setSynth(s => ({ ...s, phase: 'generation' }));
      const reponse = await onSynthese(prompt);
      if (!reponse || !reponse.trim()) {
        throw new Error('Le modèle n\'a pas renvoyé de synthèse (génération interrompue ou modèle non configuré).');
      }
      setSynth(s => ({
        ...s,
        phase: 'pret',
        note: assemblerNote(selected.nom, reponse, textes),
        nbAnalyses: charges.length,
        nbEchecs: textes.length - charges.length,
        nbTronques: charges.filter(t => t.tronque).length,
      }));
    } catch (e) {
      setSynth(s => ({ ...s, phase: 'erreur', erreur: e instanceof Error ? e.message : String(e) }));
    }
  };

  const telechargerWord = async () => {
    if (!selected || !synth.note) return;
    try { await generateDocx(synth.note, nomFichierNote(selected.nom)); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };

  const copierNote = async () => {
    try { await navigator.clipboard.writeText(synth.note); setInfo('Note copiée dans le presse-papiers.'); }
    catch { setError('Copie impossible : sélectionnez le texte manuellement.'); }
  };

  const renderSynthese = () => {
    if (synth.phase === 'idle') return null;
    return (
      <div className="veille-synth" role="region" aria-label="Synthèse de veille">
        {synth.phase === 'config' && (
          <>
            <h4>✍️ Note de veille — {nbChoisis} texte{nbChoisis > 1 ? 's' : ''}</h4>
            <label>Consigne (facultatif)
              <input value={synth.consigne} maxLength={300}
                     placeholder="Ex. pour un DRH : insister sur les obligations de l'employeur"
                     onChange={e => setSynth(s => ({ ...s, consigne: e.target.value }))} />
            </label>
            <p className="veille-hint">
              Le texte intégral de chaque élément est chargé depuis Légifrance / Judilibre, puis envoyé au modèle
              (les textes très longs sont tronqués). La synthèse s'affiche aussi dans le chat.
            </p>
            <div className="veille-form__actions">
              <button className="veille-btn veille-btn--primary" onClick={lancerSynthese}>Générer la synthèse</button>
              <button className="veille-btn" onClick={() => setSynth(SYNTH_INIT)}>Annuler</button>
            </div>
          </>
        )}
        {synth.phase === 'chargement' && (
          <p role="status">Chargement des textes… {synth.faits} / {synth.total}</p>
        )}
        {synth.phase === 'generation' && (
          <p role="status">Rédaction de la synthèse par le modèle… (elle s'affiche aussi dans le chat)</p>
        )}
        {synth.phase === 'erreur' && (
          <>
            <div className="veille-error" role="alert">{synth.erreur}</div>
            <div className="veille-form__actions">
              <button className="veille-btn" onClick={() => setSynth(s => ({ ...s, phase: 'config', erreur: null }))}>Réessayer</button>
              <button className="veille-btn" onClick={() => setSynth(SYNTH_INIT)}>Fermer</button>
            </div>
          </>
        )}
        {synth.phase === 'pret' && (
          <>
            <h4>✅ Note de veille prête — {synth.nbAnalyses} texte{synth.nbAnalyses > 1 ? 's' : ''} analysé{synth.nbAnalyses > 1 ? 's' : ''}</h4>
            {synth.nbEchecs > 0 && (
              <div className="veille-warn">{synth.nbEchecs} texte{synth.nbEchecs > 1 ? 's' : ''} n'a / n'ont pas pu être chargé{synth.nbEchecs > 1 ? 's' : ''} (listé{synth.nbEchecs > 1 ? 's' : ''} en fin de note).</div>
            )}
            {synth.nbTronques > 0 && (
              <div className="veille-warn">{synth.nbTronques} texte{synth.nbTronques > 1 ? 's' : ''} tronqué{synth.nbTronques > 1 ? 's' : ''} : seule la première partie a été transmise au modèle.</div>
            )}
            <div className="veille-synth__preview bubble-markdown">
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={ARTIFACT_MD_COMPONENTS}>
                {normalizeHeadings(normalizeLatex(synth.note))}
              </ReactMarkdown>
            </div>
            <div className="veille-form__actions">
              <button className="veille-btn veille-btn--primary" onClick={telechargerWord}>📄 Télécharger en Word</button>
              <button className="veille-btn" onClick={copierNote}>Copier</button>
              <button className="veille-btn" onClick={() => setSynth(SYNTH_INIT)}>Fermer</button>
            </div>
          </>
        )}
      </div>
    );
  };

  // ── Formulaire ─────────────────────────────────────────────────────────────

  const setC = (patch: Partial<VeilleCriteres>) => setForm(f => f && ({ ...f, criteres: { ...f.criteres, ...patch } }));
  const toggleIn = (list: string[], v: string) => list.includes(v) ? list.filter(x => x !== v) : [...list, v];

  const renderForm = () => {
    if (!form) return null;
    const c = form.criteres;
    const lf = c.source === 'legifrance';
    const canSave = form.nom.trim().length > 0 && c.query.trim().length > 0 && !busy;
    return (
      <form className="veille-form" onSubmit={e => { e.preventDefault(); if (canSave) submit(); }} aria-label="Veille">
        <h3>{editingId ? 'Modifier la veille' : 'Nouvelle veille'}</h3>
        <label>Nom
          <input value={form.nom} maxLength={80} placeholder="Ex. Télétravail — textes officiels"
                 onChange={e => setForm({ ...form, nom: e.target.value })} />
        </label>
        <label>Source
          <select value={c.source} onChange={e => setC({ source: e.target.value as VeilleCriteres['source'], fond: e.target.value === 'legifrance' ? 'LODA_DATE' : null, natures: [], code: null, juridictions: [] })}>
            <option value="legifrance">Légifrance (textes, codes, jurisprudence)</option>
            <option value="judilibre">Judilibre (décisions de la Cour de cassation et des juridictions judiciaires)</option>
          </select>
        </label>
        {lf && (
          <label>Fonds documentaire
            <select value={c.fond ?? 'LODA_DATE'} onChange={e => setC({ fond: e.target.value, natures: [], code: null })}>
              {(limites?.fonds ?? FOND_OPTIONS.map(f => f.value)).map(v => (
                <option key={v} value={v}>{FOND_OPTIONS.find(f => f.value === v)?.label ?? v}</option>
              ))}
            </select>
          </label>
        )}
        <label>Mots-clés <span className="veille-req">(obligatoire)</span>
          <input value={c.query} maxLength={300} placeholder="Ex. télétravail, frais professionnels"
                 onChange={e => setC({ query: e.target.value })} />
        </label>
        {lf && c.fond === 'CODE_DATE' && (
          <label>Code
            <input list="veille-codes" value={c.code ?? ''} placeholder="Tous les codes"
                   onChange={e => setC({ code: e.target.value || null })} />
            <datalist id="veille-codes">{CODES_VEILLE.map(n => <option key={n} value={n} />)}</datalist>
          </label>
        )}
        {lf && FONDS_AVEC_NATURE.includes(c.fond ?? '') && (
          <fieldset><legend>Nature du texte (facultatif)</legend>
            {NATURES.map(n => (
              <label key={n.value} className="veille-check">
                <input type="checkbox" checked={c.natures.includes(n.value)} onChange={() => setC({ natures: toggleIn(c.natures, n.value) })} /> {n.label}
              </label>
            ))}
          </fieldset>
        )}
        {!lf && (
          <fieldset><legend>Juridictions (par défaut : Cour de cassation)</legend>
            {JURIDICTIONS.map(j => (
              <label key={j.value} className="veille-check">
                <input type="checkbox" checked={c.juridictions.includes(j.value)} onChange={() => setC({ juridictions: toggleIn(c.juridictions, j.value) })} /> {j.label}
              </label>
            ))}
          </fieldset>
        )}
        <label>Fréquence de vérification
          <select value={form.frequence} onChange={e => setForm({ ...form, frequence: Number(e.target.value) })}>
            {FREQUENCES.filter(f => !limites || (f.value >= limites.frequence_min_h && f.value <= limites.frequence_max_h)).map(f => (
              <option key={f.value} value={f.value}>{f.label}</option>
            ))}
          </select>
        </label>
        <label>Chercher à partir du <span className="veille-hint">(par défaut : 14 jours en arrière)</span>
          <input type="date" value={form.depuis} onChange={e => setForm({ ...form, depuis: e.target.value })} />
        </label>
        <div className="veille-form__actions">
          <button type="submit" className="veille-btn veille-btn--primary" disabled={!canSave}>{busy ? 'Enregistrement…' : 'Enregistrer'}</button>
          <button type="button" className="veille-btn" onClick={() => { setForm(null); setEditingId(null); }}>Annuler</button>
        </div>
      </form>
    );
  };

  // ── Rendu ──────────────────────────────────────────────────────────────────

  return (
    <div className="veille-panel" data-testid="veille-panel">
      <div className="veille-panel__head">
        <h2>🔔 Veille juridique</h2>
        <button className="veille-btn" onClick={onClose} aria-label="Fermer la veille">✕</button>
      </div>

      {error && <div className="veille-error" role="alert">{error}</div>}
      {info && <div className="veille-info" role="status">{info}</div>}

      <div className="veille-panel__body">
        <aside className="veille-list">
          <button className="veille-btn veille-btn--primary" onClick={startCreate} disabled={!!limites && veilles.length >= limites.max_veilles}>
            + Nouvelle veille
          </button>
          {loaded && veilles.length === 0 && !form && (
            <p className="veille-empty">
              Une veille rejoue vos recherches à intervalles réguliers et vous signale les textes et décisions
              <strong> nouveaux</strong>. Elle ne tourne que lorsque l'application est ouverte ; au démarrage,
              elle rattrape la période manquée.
            </p>
          )}
          {veilles.map(v => (
            <button key={v.id} className={`veille-item${v.id === selectedId ? ' veille-item--active' : ''}${v.active ? '' : ' veille-item--paused'}`}
                    onClick={() => { setSelectedId(v.id); setForm(null); setInfo(null); }}>
              <span className="veille-item__name">{v.nom}</span>
              {v.non_lus > 0 && <span className="veille-badge" aria-label={`${v.non_lus} non lus`}>{v.non_lus}</span>}
              {!v.active && <span className="veille-item__state">en pause</span>}
              {v.last_error && <span className="veille-item__state veille-item__state--err" title={v.last_error}>⚠</span>}
            </button>
          ))}
        </aside>

        <section className="veille-detail">
          {form ? renderForm() : selected ? (
            <>
              <div className="veille-detail__head">
                <h3>{selected.nom}</h3>
                <div className="veille-criteres">{describeCriteres(selected.criteres)}</div>
                <div className="veille-meta">
                  Vérifiée {relativeTime(selected.last_success_at)} · toutes les {selected.frequence_heures} h
                  {!selected.active && ' · en pause'}
                </div>
                {selected.last_error && <div className="veille-error" role="alert">{selected.last_error}</div>}
                {selected.last_notice && <div className="veille-warn">{selected.last_notice}</div>}
                <div className="veille-detail__actions">
                  <button className="veille-btn" onClick={runNow} disabled={busy}>{busy ? 'Vérification…' : 'Vérifier maintenant'}</button>
                  <button className="veille-btn" onClick={markAllRead} disabled={selected.non_lus === 0}>Tout marquer comme lu</button>
                  <button className="veille-btn" onClick={() => toggleActive(selected)}>{selected.active ? 'Mettre en pause' : 'Reprendre'}</button>
                  <button className="veille-btn" onClick={() => startEdit(selected)}>Modifier</button>
                  <button className="veille-btn veille-btn--danger" onClick={() => remove(selected)}>Supprimer</button>
                </div>
                <label className="veille-check">
                  <input type="checkbox" checked={onlyUnread} onChange={e => setOnlyUnread(e.target.checked)} /> Non lus seulement
                </label>
              </div>

              {resultats.length > 0 && onSynthese && (
                <div className="veille-select-bar">
                  <label className="veille-check">
                    <input type="checkbox" checked={tousChoisis} onChange={toutChoisir} disabled={synthEnCours} />
                    {tousChoisis ? 'Tout désélectionner' : `Sélectionner les ${Math.min(MAX_TEXTES, resultats.length)} plus récents`}
                  </label>
                  <span className="veille-select-bar__count">
                    {nbChoisis} sélectionné{nbChoisis > 1 ? 's' : ''} (max. {MAX_TEXTES})
                  </span>
                  <button className="veille-btn veille-btn--primary"
                          disabled={nbChoisis === 0 || nbChoisis > MAX_TEXTES || synthEnCours}
                          onClick={() => setSynth(s => ({ ...SYNTH_INIT, phase: 'config', consigne: s.consigne }))}>
                    ✍️ Synthétiser…
                  </button>
                </div>
              )}
              {renderSynthese()}

              {resultats.length === 0 ? (
                <p className="veille-empty">
                  {onlyUnread ? 'Aucun résultat non lu.' : selected.last_success_at ? 'Rien de nouveau pour le moment.' : 'Pas encore vérifiée : cliquez sur « Vérifier maintenant ».'}
                </p>
              ) : (
                <ul className="veille-results">
                  {resultats.map(r => (
                    <li key={r.id} className={r.vu ? 'veille-result' : 'veille-result veille-result--new'}>
                      {onSynthese && (
                        <input type="checkbox" className="veille-result__check" checked={choisis.has(r.id)}
                               onChange={() => toggleChoix(r.id)} disabled={synthEnCours}
                               aria-label={`Sélectionner pour la synthèse : ${r.titre}`} />
                      )}
                      <button onClick={() => open(r)} title="Ouvrir dans la visionneuse">
                        <span className="veille-result__title">{r.titre}</span>
                        <span className="veille-result__meta">
                          {[r.nature, formatTexteDate(r.date_texte)].filter(Boolean).join(' · ')}
                          {' · détecté '}{relativeTime(r.detecte_le)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              {resultats.length < totalRes && (
                <button className="veille-btn" onClick={() => loadResultats(selected.id, onlyUnread, resultats.length)}>
                  Voir plus ({totalRes - resultats.length})
                </button>
              )}
            </>
          ) : (
            loaded && veilles.length > 0 && <p className="veille-empty">Sélectionnez une veille.</p>
          )}
        </section>
      </div>
    </div>
  );
}
