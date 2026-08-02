// ============================================================================
// Juridix — Espace "Recherche juridique" — Hook useDossiers
// ============================================================================
// État et logique des dossiers / cartable : multi-dossiers (créer, renommer,
// supprimer, changer d'actif), ajout/retrait de documents, ouverture d'un
// document stocké sans appel réseau, synthèse LLM du dossier, export
// Markdown.
//
// Dépendances externes passées en paramètres — c'est le point de couplage
// avec le viewer documenté dans le plan de refactor initial :
// `openDossierItem` ouvre un document du dossier exactement comme le ferait
// une recherche (empile `viewedDoc` courant dans `docStack`, vide l'erreur).
//
// Extrait de JuridiquePanel.tsx (étape 4 du refactor — cf. discussion du
// 05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { useState, useCallback } from 'react';
import { API_BASE } from '../../../constants';
import { extractTextContent } from '../content';
import { stripHtml } from '../content/html-helpers';
import { safeSetJSON, pruneHeavyFieldsForStorage } from '../storage';
import type { Dossier, DossierItem, DossierAnalysis, ViewedDoc } from '../types';

export function useDossiers(
  viewedDoc: ViewedDoc | null,
  setViewedDoc: (doc: ViewedDoc | null) => void,
  setDocStack: React.Dispatch<React.SetStateAction<ViewedDoc[]>>,
  setDocError: (err: string | null) => void,
  onSendToChat: (message: string) => Promise<string | null>,
  onShowInChat: (message: string) => void,
) {
  const [dossiers, setDossiers] = useState<Dossier[]>(() => {
    try {
      const saved = localStorage.getItem('juridique_dossiers');
      if (saved) return JSON.parse(saved) as Dossier[];
      // Migration : ancien dossier unique
      const legacy = localStorage.getItem('juridique_dossier');
      if (legacy) {
        const items = JSON.parse(legacy) as DossierItem[];
        if (items.length > 0) {
          return [{ id: 'dossier_default', nom: 'Dossier principal', createdAt: Date.now(), items }];
        }
      }
      return [];
    } catch { return []; }
  });
  const [activeDossierId, setActiveDossierId] = useState<string | null>(() => {
    try {
      const saved = localStorage.getItem('juridique_dossiers');
      if (saved) {
        const ds = JSON.parse(saved) as Dossier[];
        return ds[0]?.id ?? null;
      }
      const legacy = localStorage.getItem('juridique_dossier');
      if (legacy && JSON.parse(legacy).length > 0) return 'dossier_default';
    } catch {}
    return null;
  });
  const [showDossier, setShowDossier] = useState(false);
  const [dossierView, setDossierView] = useState<'list' | 'detail'>('list');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');

  /** Dossier actif courant */
  const activeDossier = dossiers.find((d) => d.id === activeDossierId) ?? null;
  /** Raccourci items du dossier actif (rétrocompat) */
  const dossier = activeDossier?.items ?? [];

  /** Persiste la liste de dossiers */
  const saveDossiers = (ds: Dossier[]) => safeSetJSON('juridique_dossiers', ds);

  const [dossierLlmLoading, setDossierLlmLoading] = useState(false);
  const [dossierLlmMode, setDossierLlmMode] = useState<'synthese' | 'contradictoire' | 'points_cles' | null>(null);

  const isInDossier = useCallback((id: string) =>
    dossier.some((d) => d.id === id), [dossier]);

  /** Crée un nouveau dossier et le rend actif */
  const createDossier = useCallback(() => {
    const id = `dossier_${Date.now()}`;
    const nom = `Nouveau dossier`;
    const nd: Dossier = { id, nom, createdAt: Date.now(), items: [] };
    setDossiers((prev) => { const next = [...prev, nd]; saveDossiers(next); return next; });
    setActiveDossierId(id);
    setDossierView('detail');
  }, []);

  /** Renomme un dossier */
  const renameDossier = useCallback((id: string, nom: string) => {
    setDossiers((prev) => { const next = prev.map((d) => d.id === id ? { ...d, nom: nom.trim() || d.nom } : d); saveDossiers(next); return next; });
    setRenamingId(null);
  }, []);

  /** Supprime un dossier entier */
  const deleteDossier = useCallback((id: string) => {
    setDossiers((prev) => {
      const next = prev.filter((d) => d.id !== id);
      saveDossiers(next);
      if (activeDossierId === id) setActiveDossierId(next[0]?.id ?? null);
      return next;
    });
  }, [activeDossierId]);

  const toggleDossier = useCallback((doc: ViewedDoc) => {
    setDossiers((prev) => {
      // Si aucun dossier actif, en créer un à la volée
      let targetId = activeDossierId;
      let ds = prev;
      if (!targetId || !prev.find((d) => d.id === targetId)) {
        const id = `dossier_${Date.now()}`;
        const nd: Dossier = { id, nom: 'Dossier principal', createdAt: Date.now(), items: [] };
        ds = [...prev, nd];
        targetId = id;
        setActiveDossierId(id);
      }
      const item: DossierItem = {
        id: doc.id,
        titre: doc.titre.split('\n')[0],
        source: doc.source,
        type: doc.type,
        contenu: doc.contenu,
        addedAt: Date.now(),
        raw: pruneHeavyFieldsForStorage(doc.raw),
      };
      const next = ds.map((d) => {
        if (d.id !== targetId) return d;
        const exists = d.items.some((it) => it.id === doc.id);
        return { ...d, items: exists
          ? d.items.filter((it) => it.id !== doc.id)
          : [...d.items, item].slice(0, 30)
        };
      });
      saveDossiers(next);
      return next;
    });
  }, [activeDossierId]);

  /** Ouvre un document stocké dans le dossier sans appel réseau */
  const openDossierItem = useCallback((item: DossierItem) => {
    const content_ = extractTextContent(item.raw, item.type as ViewedDoc['type'], item.titre);
    const contenuLlm = stripHtml(content_.body);
    setDocStack((prev) => viewedDoc ? [...prev, viewedDoc] : prev);
    setViewedDoc({
      id: item.id,
      titre: item.titre,
      content: content_,
      contenu: contenuLlm,
      source: item.source,
      type: item.type as ViewedDoc['type'],
      raw: item.raw,
    });
    setDocError(null);
    setShowDossier(false);
  }, [viewedDoc, setDocStack, setViewedDoc, setDocError]);

  const removeDossierItem = useCallback((id: string) => {
    if (!activeDossierId) return;
    setDossiers((prev) => {
      const next = prev.map((d) => d.id !== activeDossierId ? d : { ...d, items: d.items.filter((it) => it.id !== id) });
      saveDossiers(next);
      return next;
    });
  }, [activeDossierId]);

  const clearDossier = useCallback(() => {
    if (!activeDossierId) return;
    setDossiers((prev) => {
      const next = prev.map((d) => d.id !== activeDossierId ? d : { ...d, items: [] });
      saveDossiers(next);
      return next;
    });
  }, [activeDossierId]);

  const sendDossierToLlm = useCallback(async (mode: 'synthese' | 'contradictoire' | 'points_cles') => {
    if (dossier.length === 0 || !activeDossierId) return;
    setDossierLlmLoading(true);
    setDossierLlmMode(mode);
    try {
      // On réutilise envoyer-chat côté backend pour le formatage du prompt
      // (un par mode) ; le message part désormais dans le chat principal
      // partagé, ce qui permet à la réponse d'être détectée automatiquement
      // comme artefact (ArtifactsPanel) au lieu d'un mini-fil isolé.
      const res = await fetch(`${API_BASE}/api/juridique/envoyer-chat`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          mode: `dossier_${mode}`,
          dossier: dossier.map((d) => ({ titre: d.titre, source: d.source, type: d.type, contenu: d.contenu })),
        }),
      });
      const data = await res.json();
      if (data.message) {
        // `data.message` n'est QUE le prompt formaté côté backend (cf.
        // envoyer_chat dans routes.rs, qui ne fait qu'assembler le texte à
        // envoyer au LLM, sans l'appeler lui-même). La vraie synthèse n'existe
        // qu'une fois le streaming terminé côté chat principal — `onSendToChat`
        // l'attend et nous la renvoie. C'est CETTE réponse (et non le prompt)
        // qu'il faut archiver : sinon deux analyses du même mode sur le même
        // dossier produisent un prompt identique et donc une entrée d'historique
        // indiscernable, et son contenu (corpus brut) n'a pas la mise en forme
        // (titres, listes, gras) de la vraie réponse du LLM.
        const finalResponse = await onSendToChat(data.message);
        if (finalResponse) {
          // Rattache la réponse au dossier lui-même (pas seulement au chat, qui
          // repart à zéro à la prochaine analyse — cf. sendMessage côté
          // App.tsx) : historique daté, la plus récente en dernier, plafonné
          // pour ne pas faire grossir indéfiniment le localStorage (mêmes
          // ordres de grandeur que le plafond sur `items`, cf. toggleDossier).
          const entry: DossierAnalysis = {
            id: `analyse_${Date.now()}`,
            mode,
            message: finalResponse,
            createdAt: Date.now(),
            itemTitres: dossier.map((d) => d.titre),
          };
          setDossiers((prev) => {
            const next = prev.map((d) => d.id !== activeDossierId
              ? d
              : { ...d, analyses: [...(d.analyses ?? []), entry].slice(-20) });
            saveDossiers(next);
            return next;
          });
        }
      }
    } catch (e) {
      console.error('[Dossier] sendToLlm:', e);
    } finally {
      setDossierLlmLoading(false);
      setDossierLlmMode(null);
    }
  }, [dossier, activeDossierId, onSendToChat]);

  /** Retire une analyse de l'historique du dossier actif. */
  const removeDossierAnalysis = useCallback((analysisId: string) => {
    if (!activeDossierId) return;
    setDossiers((prev) => {
      const next = prev.map((d) => d.id !== activeDossierId
        ? d
        : { ...d, analyses: (d.analyses ?? []).filter((a) => a.id !== analysisId) });
      saveDossiers(next);
      return next;
    });
  }, [activeDossierId]);

  /** Réaffiche une analyse déjà générée dans le chat, SANS réappeler le LLM
   *  (relecture d'un historique déjà stocké, pas une nouvelle demande —
   *  `onSendToChat` enverrait ce texte comme une nouvelle question à
   *  l'assistant, ce qui n'a pas de sens ici). */
  const reopenDossierAnalysis = useCallback((analysis: DossierAnalysis) => {
    onShowInChat(analysis.message);
  }, [onShowInChat]);

  const exportDossierMarkdown = useCallback(() => {
    if (dossier.length === 0) return;
    const md = dossier.map((d, i) =>
      `# ${i + 1}. ${d.titre}\n*Source : ${d.source} · Type : ${d.type}*\n\n${d.contenu}`
    ).join('\n\n---\n\n');
    const blob = new Blob([md], { type: 'text/markdown' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `dossier_juridique_${new Date().toISOString().slice(0, 10)}.md`;
    a.click();
  }, [dossier]);

  return {
    dossiers, activeDossierId, setActiveDossierId,
    showDossier, setShowDossier,
    dossierView, setDossierView,
    renamingId, setRenamingId,
    renameValue, setRenameValue,
    activeDossier, dossier,
    dossierLlmLoading, dossierLlmMode,
    isInDossier,
    createDossier, renameDossier, deleteDossier, toggleDossier,
    openDossierItem, removeDossierItem, clearDossier,
    sendDossierToLlm, exportDossierMarkdown,
    removeDossierAnalysis, reopenDossierAnalysis,
  };
}
