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
// Espace "Recherche juridique" — Hook useAnnotations
// ============================================================================
// État et logique des annotations/surlignages sur le document affiché :
// persistance localStorage, sélection de texte → popover, CRUD annotation,
// scroll-vers-annotation, et surlignage du HTML rendu (getAnnotatedHtml).
//


import { useState, useCallback, useRef, type RefObject } from 'react';
import { safeSetJSON } from '../storage';
import type { Annotation, AnnotationColor, ViewedDoc } from '../types';

export function useAnnotations(
  viewedDoc: ViewedDoc | null,
  viewerContentRef: RefObject<HTMLDivElement>,
) {
  const [annotations, setAnnotations] = useState<Annotation[]>(() => {
    try { return JSON.parse(localStorage.getItem('juridique_annotations') || '[]'); }
    catch { return []; }
  });
  const [pendingAnnotation, setPendingAnnotation] = useState<{
    text: string; rangeStart: number; rangeEnd: number; x: number; y: number;
  } | null>(null);
  const [annotationNote, setAnnotationNote] = useState('');
  const [annotationColor, setAnnotationColor] = useState<AnnotationColor>('yellow');
  const [showAnnotations, setShowAnnotations] = useState(false);
  const annPopoverRef = useRef<HTMLDivElement>(null);

  const handleTextSelection = useCallback((e: React.MouseEvent) => {
    if (!viewedDoc) return;
    // Ne pas fermer la modale si le clic vient du popover lui-même
    if (annPopoverRef.current?.contains(e.target as Node)) return;
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.toString().trim()) {
      setPendingAnnotation(null);
      return;
    }
    // Normalise les espaces et sauts de ligne de la sélection DOM
    const selectedText = sel.toString().replace(/\s+/g, ' ').trim();
    if (selectedText.length < 3 || selectedText.length > 1000) return;

    // Position du popover (fixe dans le viewport)
    const range = sel.getRangeAt(0);
    const rect = range.getBoundingClientRect();

    // Retrouver le texte dans le contenu brut (sans HTML) pour rangeStart/End
    const plainContent = viewedDoc.contenu.replace(/\s+/g, ' ');
    const start = plainContent.indexOf(selectedText);
    const end = start >= 0 ? start + selectedText.length : -1;

    setPendingAnnotation({
      text: selectedText,
      rangeStart: start,
      rangeEnd: end,
      x: rect.left + rect.width / 2,
      y: rect.top - 8,
    });
    setAnnotationNote('');
  }, [viewedDoc]);

  const saveAnnotation = useCallback(() => {
    if (!pendingAnnotation || !viewedDoc) return;
    const ann: Annotation = {
      id: `ann_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
      docId: viewedDoc.id,
      text: pendingAnnotation.text,
      note: annotationNote,
      color: annotationColor,
      createdAt: Date.now(),
      rangeStart: pendingAnnotation.rangeStart,
      rangeEnd: pendingAnnotation.rangeEnd,
    };
    setAnnotations((prev) => {
      const next = [ann, ...prev];
      safeSetJSON('juridique_annotations', next);
      return next;
    });
    setPendingAnnotation(null);
    setAnnotationNote('');
  }, [pendingAnnotation, viewedDoc, annotationNote, annotationColor]);

  /** Scrolle vers le <mark> correspondant à l'annotation dans le viewer */
  const scrollToAnnotation = useCallback((annId: string) => {
    const container = viewerContentRef.current;
    if (!container) return;
    const mark = container.querySelector(`mark[data-ann-id="${annId}"]`) as HTMLElement | null;
    if (!mark) return;
    // Fermer le panneau annotations pour révéler le texte
    setShowAnnotations(false);
    // Flash + scroll
    mark.scrollIntoView({ behavior: 'smooth', block: 'center' });
    mark.classList.add('jur-ann--flash');
    setTimeout(() => mark.classList.remove('jur-ann--flash'), 1400);
  }, [viewerContentRef]);

  const deleteAnnotation = useCallback((id: string) => {
    setAnnotations((prev) => {
      const next = prev.filter((a) => a.id !== id);
      safeSetJSON('juridique_annotations', next);
      return next;
    });
  }, []);

  const changeAnnotationColor = useCallback((id: string, color: AnnotationColor) => {
    setAnnotations((prev) => {
      const next = prev.map((a) => a.id === id ? { ...a, color } : a);
      safeSetJSON('juridique_annotations', next);
      return next;
    });
  }, []);

  const [editingAnnotationId, setEditingAnnotationId] = useState<string | null>(null);
  const [editingAnnotationNote, setEditingAnnotationNote] = useState('');

  const startEditAnnotation = useCallback((ann: Annotation) => {
    setEditingAnnotationId(ann.id);
    setEditingAnnotationNote(ann.note);
  }, []);

  const saveEditAnnotation = useCallback((id: string) => {
    setAnnotations((prev) => {
      const next = prev.map((a) => a.id === id ? { ...a, note: editingAnnotationNote } : a);
      safeSetJSON('juridique_annotations', next);
      return next;
    });
    setEditingAnnotationId(null);
    setEditingAnnotationNote('');
  }, [editingAnnotationNote]);

  const cancelEditAnnotation = useCallback(() => {
    setEditingAnnotationId(null);
    setEditingAnnotationNote('');
  }, []);

  const currentDocAnnotations = viewedDoc
    ? annotations.filter((a) => a.docId === viewedDoc.id)
    : [];

  const getAnnotatedHtml = useCallback((html: string, anns: Annotation[]): string => {
    if (anns.length === 0) return html;

    // Construit un arbre DOM temporaire
    const tpl = document.createElement('div');
    tpl.innerHTML = html;

    for (let i = 0; i < anns.length; i++) {
      const ann = anns[i];
      if (!ann.text || ann.text.length < 3) continue;
      const needle = ann.text.replace(/\s+/g, ' ').trim();
      const annNum = i + 1;
      const markClass = `jur-ann jur-ann--${ann.color}`;
      const markTitle = (ann.note || ann.text).slice(0, 80).replace(/"/g, '&quot;');

      // Collecte tous les nœuds texte sous tpl
      const walker = document.createTreeWalker(tpl, NodeFilter.SHOW_TEXT);
      const textNodes: Text[] = [];
      let node: Node | null;
      while ((node = walker.nextNode())) textNodes.push(node as Text);

      // Concatène tous les textes visibles pour localiser needle
      const fullText = textNodes.map((n) => n.nodeValue || '').join('');
      const normalFull = fullText.replace(/\s+/g, ' ');
      const normalNeedle = needle.replace(/\s+/g, ' ');
      let pos = normalFull.indexOf(normalNeedle);
      if (pos < 0) continue; // texte introuvable dans ce fragment

      // Traduit la position dans le texte normalisé → offset dans les nœuds réels
      // en parcourant les nœuds jusqu'à couvrir [pos, pos+len]
      let charCount = 0;
      let startNode: Text | null = null, startOff = 0;
      let endNode: Text | null = null, endOff = 0;
      const len = normalNeedle.length;

      for (const tn of textNodes) {
        const val = tn.nodeValue || '';
        const normVal = val.replace(/\s+/g, ' ');
        const nodeStart = charCount;
        const nodeEnd   = charCount + normVal.length;

        if (!startNode && nodeEnd > pos) {
          // Calcule l'offset réel dans le nœud (en tenant compte de la normalisation)
          // Approx : utilise le ratio position/longueur
          const localPos = pos - nodeStart;
          startNode = tn;
          startOff = Math.min(localPos, val.length);
        }
        if (!endNode && nodeEnd >= pos + len) {
          const localEnd = (pos + len) - nodeStart;
          endNode = tn;
          endOff = Math.min(localEnd, val.length);
        }
        charCount += normVal.length;
        if (startNode && endNode) break;
      }

      if (!startNode || !endNode) continue;

      try {
        const range = document.createRange();
        range.setStart(startNode, startOff);
        range.setEnd(endNode, endOff);

        const mark = document.createElement('mark');
        mark.className = markClass;
        mark.title = markTitle;
        mark.dataset.annNum = String(annNum);
        mark.dataset.annId = ann.id;
        range.surroundContents(mark);
      } catch {
        // surroundContents échoue si la sélection traverse des nœuds éléments
        // Fallback : on enveloppe chaque nœud texte impliqué séparément
        try {
          for (const tn of [startNode, endNode]) {
            const mark2 = document.createElement('mark');
            mark2.className = markClass;
            mark2.title = markTitle;
            mark2.dataset.annNum = String(annNum);
            mark2.dataset.annId = ann.id;
            const r2 = document.createRange();
            if (tn === startNode && tn === endNode) {
              r2.setStart(tn, startOff); r2.setEnd(tn, endOff);
            } else if (tn === startNode) {
              r2.setStart(tn, startOff); r2.setEnd(tn, (tn.nodeValue || '').length);
            } else {
              r2.setStart(tn, 0); r2.setEnd(tn, endOff);
            }
            r2.surroundContents(mark2);
          }
        } catch { /* on abandonne pour cette annotation */ }
      }
    }

    return tpl.innerHTML;
  }, []);

  return {
    annotations,
    pendingAnnotation, setPendingAnnotation,
    annotationNote, setAnnotationNote,
    annotationColor, setAnnotationColor,
    showAnnotations, setShowAnnotations,
    annPopoverRef,
    handleTextSelection,
    saveAnnotation,
    scrollToAnnotation,
    deleteAnnotation,
    changeAnnotationColor,
    editingAnnotationId, editingAnnotationNote, setEditingAnnotationNote,
    startEditAnnotation, saveEditAnnotation, cancelEditAnnotation,
    currentDocAnnotations,
    getAnnotatedHtml,
  };
}
