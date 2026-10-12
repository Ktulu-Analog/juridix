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
// Espace "Recherche juridique" — Hook useDocumentViewer
// ============================================================================
// État et logique de la visionneuse de document : ouverture d'un document
// (openDocument, routée par fond/source vers le bon endpoint backend), pile
// de navigation (goBack/closeDocument), table des matières flottante
// (scroll-spy), timeline des versions légales, prévisualisation au survol
// des liens de citation, et export Markdown.
//
// ============================================================================

import { useState, useCallback, useRef, useEffect, type MouseEvent as ReactMouseEvent } from 'react';
import { API_BASE } from '../../../constants';
import {
  isTextId, stripHtml, htmlToPlainBlocks, resolveInlineCitations, countCitableArticleIds, DEFAULT_MAX_INLINE_CITATIONS,
} from '../content/html-helpers';
import { extractTextContent } from '../content';
import { extractArticleInlineText } from '../content/extract-loda';
import { extractTitle, extractResultId } from '../search-helpers';
import type {
  SearchResult, Source, Fond, DocType, ViewedDoc, TocTreeNode, TimelineVersion, DossierLegislatifRef,
} from '../types';

// Mapping fond → endpoint backend pour les fonds spéciaux (listes)
export function fondToEndpointInfo(fond: Fond, id: string, idIsTextId: boolean): { endpoint: string; type: DocType } {
  const base = `${API_BASE}/api/juridique`;
  switch (fond) {
    case 'CODE_DATE':
    case 'CODE_ETAT':
      return { endpoint: `${base}/code/${encodeURIComponent(id)}`, type: 'code' };

    case 'LODA_DATE':
    case 'LODA_ETAT':
      if (idIsTextId) {
        return { endpoint: `${base}/loda/${encodeURIComponent(id)}`, type: 'loda' };
      }
      return { endpoint: `${base}/article/${encodeURIComponent(id)}`, type: 'article' };

    case 'JORF':
      if (idIsTextId) {
        return { endpoint: `${base}/jorf-part/${encodeURIComponent(id)}`, type: 'loda' };
      }
      return { endpoint: `${base}/article/${encodeURIComponent(id)}`, type: 'article' };

    case 'JURI':
    case 'CETAT':
    case 'CONSTIT':
    case 'JUFI':
      return { endpoint: `${base}/juri/${encodeURIComponent(id)}`, type: 'juri' };

    case 'KALI':
      return { endpoint: `${base}/kali/${encodeURIComponent(id)}`, type: 'kali' };

    case 'CNIL':
      return { endpoint: `${base}/cnil/${encodeURIComponent(id)}`, type: 'cnil' };

    case 'CIRC':
      return { endpoint: `${base}/circulaire/${encodeURIComponent(id)}`, type: 'circ' };

    case 'ACCO':
      return { endpoint: `${base}/acco/${encodeURIComponent(id)}`, type: 'acco' };

    case 'BOCC':
      return { endpoint: `${base}/bocc-texts-list`, type: 'bocc' };

    case 'DEBAT':
      return { endpoint: `${base}/debat/${encodeURIComponent(id)}`, type: 'debat' };

    case 'QUESTION_ECRITE':
      return { endpoint: `${base}/questions-ecrites-list`, type: 'question_ecrite' };

    case 'DOC_ADMIN':
      return { endpoint: `${base}/docs-admins`, type: 'doc_admin' };

    default:
      // Pour "Tous les fonds" et cas non prévus : déduire l'endpoint depuis le préfixe de l'id
      if (/^JORFTEXT|^JORFCONT/.test(id))
        return { endpoint: `${base}/jorf-part/${encodeURIComponent(id)}`, type: 'loda' };
      if (/^LEGITEXT/.test(id))
        return { endpoint: `${base}/loda/${encodeURIComponent(id)}`, type: 'loda' };
      if (/^JURITEXT/.test(id))
        return { endpoint: `${base}/juri/${encodeURIComponent(id)}`, type: 'juri' };
      if (/^KALITEXT|^KALICONT/.test(id))
        return { endpoint: `${base}/kali/${encodeURIComponent(id)}`, type: 'kali' };
      // LEGIARTI, JORFARTI, KALIARTI, LEGISCTA → article
      return { endpoint: `${base}/article/${encodeURIComponent(id)}`, type: 'article' };
  }
}

/** Formate un timestamp ms (string ou number) ou une date ISO en Date JS.
 *  Exportée : également utilisée directement par le JSX de la timeline
 *  resté dans JuridiquePanel.tsx (formatage d'affichage des dates de version). */
export function parseChronoDate(raw: string | number | undefined): Date | null {
  if (!raw) return null;
  const n = Number(raw);
  if (!isNaN(n) && n > 1000000000000) return new Date(n); // timestamp ms
  if (!isNaN(n) && n > 0) return new Date(n * 1000);       // timestamp s
  const d = new Date(raw as string);
  return isNaN(d.getTime()) ? null : d;
}

/** Extrait un texte court (≤ 400 car) à afficher dans la bulle de prévisualisation */
function extractPreviewText(raw: unknown, id: string): string {
  if (!raw || typeof raw !== 'object') return '';
  const obj = raw as Record<string, unknown>;
  // Article
  const art = obj.article as Record<string, unknown> | undefined;
  if (art) {
    const txt = stripHtml(String(art.texteHtml || art.texte || ''));
    if (txt) return txt.slice(0, 400) + (txt.length > 400 ? '…' : '');
  }
  // ConsultArticle direct (content)
  if (obj.content) {
    const txt = stripHtml(String(obj.content));
    if (txt) return txt.slice(0, 400) + (txt.length > 400 ? '…' : '');
  }
  // Decision JUDILIBRE
  if (obj.text) {
    const txt = String(obj.text).slice(0, 400);
    return txt + (String(obj.text).length > 400 ? '…' : '');
  }
  // LODA : titre + premier article
  const title = stripHtml(String(obj.title || obj.titre || ''));
  const arts = (obj.articles || []) as Array<Record<string, unknown>>;
  if (arts.length) {
    const first = stripHtml(String(arts[0].content || arts[0].texteHtml || arts[0].texte || ''));
    return [title, first].filter(Boolean).join(' — ').slice(0, 400) + '…';
  }
  // Jurisprudence Légifrance
  const dec = obj.decision as Record<string, unknown> | undefined;
  if (dec) {
    const txt = stripHtml(String(dec.texte || dec.text || ''));
    if (txt) return txt.slice(0, 400) + (txt.length > 400 ? '…' : '');
  }
  if (title) return title;
  return '';
}

export function useDocumentViewer(fond: Fond, source: Source) {
  const [viewedDoc, setViewedDoc] = useState<ViewedDoc | null>(null);
  const [loadingDoc, setLoadingDoc] = useState(false);
  const [docError, setDocError] = useState<string | null>(null);
  // Pile de navigation : documents visités avant le doc courant
  const [docStack, setDocStack] = useState<ViewedDoc[]>([]);

  // ── Navigation : retour au document précédent ───────────────────────────────
  const goBack = useCallback(() => {
    setDocStack((prev) => {
      if (prev.length === 0) return prev;
      const next = [...prev];
      const prevDoc = next.pop()!;
      setViewedDoc(prevDoc);
      setDocError(null);
      return next;
    });
  }, []);

  // ── Retour au niveau recherche : ferme le document, vide la pile de
  // navigation, la liste de résultats reprend toute la largeur. ───────────────
  const closeDocument = useCallback(() => {
    setViewedDoc(null);
    setDocStack([]);
    setDocError(null);
  }, []);

  // ── Résolution inline des citations d'article
  const inlineArticleCache = useRef<Map<string, string | null>>(new Map());
  const fetchArticleHtml = useCallback(async (id: string): Promise<string | null> => {
    const cache = inlineArticleCache.current;
    if (cache.has(id)) return cache.get(id)!;
    try {
      const res = await fetch(`${API_BASE}/api/juridique/article/${encodeURIComponent(id)}`);
      if (!res.ok) { cache.set(id, null); return null; }
      const raw = await res.json();
      const txt = raw?.error ? null : extractArticleInlineText(raw as Record<string, unknown>);
      cache.set(id, txt);
      return txt;
    } catch {
      cache.set(id, null);
      return null;
    }
  }, []);

  // ── Ouverture d'un document ─────────────────────────────────────────────────
  const openDocument = useCallback(async (
    result: SearchResult,
    resetStack = false,
    // Fond/source à utiliser pour router l'ouverture de ce document.
    overrideFond?: Fond,
    overrideSource?: Source,
  ) => {

    const isLikelyJudilibreId = /^[0-9a-f]{24}$/i.test(result.id || '');

    const effFond = overrideFond ?? (['CODE_DATE', 'CODE_ETAT', 'LODA_DATE', 'LODA_ETAT', 'JORF'].includes(fond) ? 'ALL' : fond);
    const effSource = overrideSource ?? (isLikelyJudilibreId ? 'judilibre' : source);
    if (resetStack) {
      // Ouverture depuis la liste de résultats → réinitialise la pile
      setDocStack([]);
      setViewedDoc(null);
    } else {
      // Navigation depuis un lien de citation → empile le doc courant
      setViewedDoc((current) => {
        if (current) setDocStack((prev) => [...prev, current]);
        return null;
      });
    }
    setLoadingDoc(true);
    setDocError(null);

    try {
      const isCodeFond = ['CODE_DATE', 'CODE_ETAT'].includes(effFond);
      const id = effSource === 'judilibre'
        ? (result.id || '')
        : (result._versionCid ? (result.id || '') : extractResultId(result, isCodeFond));
      if (!id) throw new Error('Identifiant introuvable');

      // Lien de version d'article
      if (result._versionCid) {
        const endpoint = `${API_BASE}/api/juridique/article-version/${encodeURIComponent(result._versionCid)}/${encodeURIComponent(id)}`;
        const res = await fetch(endpoint);
        if (!res.ok) {
          const err = await res.json().catch(() => ({}));
          throw new Error(err.error || `Erreur ${res.status}`);
        }
        const raw = await res.json();
        if (raw.error) throw new Error(raw.error);
        const titre = extractTitle(result, effSource);
        const rawContent = extractTextContent(raw, 'article', titre);
        // Résolution inline des citations
        const totalInlineArticles = countCitableArticleIds(rawContent.body);
        const content = { ...rawContent, body: await resolveInlineCitations(rawContent.body, fetchArticleHtml) };
        const contenuLlm = htmlToPlainBlocks(content.body);
        setViewedDoc({
          id, titre, content, contenu: contenuLlm, source: effSource, type: 'article', raw,
          rawBody: totalInlineArticles > 0 ? rawContent.body : undefined,
          totalInlineArticles,
          allArticlesLoaded: totalInlineArticles <= DEFAULT_MAX_INLINE_CITATIONS,
        });
        setLoadingDoc(false);
        return;
      }

      if (effSource === 'legifrance' && (effFond === 'BOCC' || effFond === 'QUESTION_ECRITE' || effFond === 'DOC_ADMIN' || effFond === 'DEBAT')) {
        const type: ViewedDoc['type'] = effFond === 'BOCC' ? 'bocc' : effFond === 'QUESTION_ECRITE' ? 'question_ecrite' : effFond === 'DEBAT' ? 'debat' : 'doc_admin';
        const titre = extractTitle(result, effSource);
        const content = extractTextContent(result, type, titre);
        const contenuLlm = htmlToPlainBlocks(content.body);
        setViewedDoc({ id, titre, content, contenu: contenuLlm, source: effSource, type, raw: result });
        setLoadingDoc(false);
        return;
      }

      let endpoint: string;
      let type: ViewedDoc['type'];

      if (effSource === 'judilibre') {
        endpoint = `${API_BASE}/api/juridique/decision/${encodeURIComponent(id)}?resolve_references=true`;
        type = 'decision';
      } else {
        // Routing complet par fond : chaque fond a son propre endpoint backend
        const info = fondToEndpointInfo(effFond as Fond, id, isTextId(id));
        endpoint = info.endpoint;
        type = info.type;
      }

      const res = await fetch(endpoint);
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error(err.error || `Erreur ${res.status}`);
      }
      const raw = await res.json();
      if (raw.error) throw new Error(raw.error);

      const titre = extractTitle(result, effSource);
      const rawContent = extractTextContent(raw, type, titre);
      // Résolution inline des citations
      const totalInlineArticles = countCitableArticleIds(rawContent.body);
      const content = { ...rawContent, body: await resolveInlineCitations(rawContent.body, fetchArticleHtml) };
      // Texte brut pour LLM = HTML strippé
      const contenuLlm = htmlToPlainBlocks(content.body);

      setViewedDoc({
        id, titre, content, contenu: contenuLlm, source: effSource, type, raw,
        rawBody: totalInlineArticles > 0 ? rawContent.body : undefined,
        totalInlineArticles,
        allArticlesLoaded: totalInlineArticles <= DEFAULT_MAX_INLINE_CITATIONS,
        dossiersLegislatifs: (raw as { dossiersLegislatifs?: DossierLegislatifRef[] })?.dossiersLegislatifs
          ?? result.dossiersLegislatifs,
      });
    } catch (e: unknown) {
      setDocError((e as Error).message || 'Erreur de chargement');
    } finally {
      setLoadingDoc(false);
    }
  }, [source, fond, fetchArticleHtml]);

  const openDossierLegislatif = useCallback(async (id: string, title?: string) => {
    setViewedDoc((current) => {
      if (current) setDocStack((prev) => [...prev, current]);
      return null;
    });
    setLoadingDoc(true);
    setDocError(null);
    try {
      const res = await fetch(`${API_BASE}/api/juridique/dossier/${encodeURIComponent(id)}`);
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error(err.error || `Erreur ${res.status}`);
      }
      const raw = await res.json();
      if (raw.error) throw new Error(raw.error);
      const titre = title || 'Dossier législatif';
      const content = extractTextContent(raw, 'dossier_legislatif', titre);
      const contenuLlm = htmlToPlainBlocks(content.body);
      setViewedDoc({
        id, titre, content, contenu: contenuLlm, source: 'legifrance', type: 'dossier_legislatif', raw,
      });
    } catch (e: unknown) {
      setDocError((e as Error).message || 'Erreur de chargement');
    } finally {
      setLoadingDoc(false);
    }
  }, []);

  // ── Sync ref pour citations : évite une closure figée dans
  // handleCitationClick (déclaré une seule fois, cf. plus bas). ──────────────
  const openDocumentRef = useRef<(r: SearchResult) => void>(() => {});
  useEffect(() => { openDocumentRef.current = openDocument; }, [openDocument]);

  // ── Plan interactif (sommaire façon Dalloz) ──────────────────────────────────
  // Partagée avec useAnnotations (scroll-vers-annotation dans le même
  // conteneur) — c'est pourquoi la ref sort de ce hook plutôt que d'y rester
  // interne.
  const viewerContentRef = useRef<HTMLDivElement>(null);
  const [tocTree, setTocTree] = useState<TocTreeNode[]>([]);
  const [activeTocId, setActiveTocId] = useState<string | null>(null);
  const [expandedTocIds, setExpandedTocIds] = useState<Set<string>>(new Set());
  const tocObserverRef = useRef<IntersectionObserver | null>(null);
  const tocParentRef = useRef<Map<string, string>>(new Map());
  const tocSectionIdsRef = useRef<string[]>([]);
  const revealTocPath = useCallback((id: string) => {
    setExpandedTocIds((prev) => {
      let current = tocParentRef.current.get(id);
      if (!current) return prev; // nœud racine : aucun ancêtre à déplier
      const next = new Set(prev);
      let changed = false;
      while (current) {
        if (!next.has(current)) { next.add(current); changed = true; }
        current = tocParentRef.current.get(current);
      }
      return changed ? next : prev;
    });
  }, []);


  const buildTocTree = useCallback((root: HTMLElement, level: number, parentId: string | null): TocTreeNode[] => {
    const nodes: TocTreeNode[] = [];
    Array.from(root.children).forEach((child) => {
      const el = child as HTMLElement;
      if (el.tagName === 'ARTICLE' && el.classList.contains('jur-article') && el.id) {
        const label = (el.querySelector('.jur-article-num')?.textContent || '').trim();
        if (!label) return;
        nodes.push({ id: el.id, label, level, type: 'article', children: [] });
        if (parentId) tocParentRef.current.set(el.id, parentId);
      } else if (el.tagName === 'SECTION' && el.classList.contains('jur-section') && el.id) {
        const titleEl = el.querySelector<HTMLElement>(':scope > .jur-section-title');
        const label = (titleEl?.textContent || '').trim();
        const children = buildTocTree(el, level + 1, el.id);
        // Une section sans titre ET sans contenu exploitable n'apporte rien
        // au plan (cas rare : section purement structurelle déjà vide côté API).
        if (!label && children.length === 0) return;
        nodes.push({ id: el.id, label: label || '(sans titre)', level, type: 'section', children });
        tocSectionIdsRef.current.push(el.id);
        if (parentId) tocParentRef.current.set(el.id, parentId);
      }
    });
    return nodes;
  }, []);

  useEffect(() => {
    const container = viewerContentRef.current;
    if (!container || !viewedDoc) {
      setTocTree([]);
      setActiveTocId(null);
      setExpandedTocIds(new Set());
      return;
    }

    const raf = requestAnimationFrame(() => {
      tocParentRef.current = new Map();
      tocSectionIdsRef.current = [];
      const body = container.querySelector<HTMLElement>('.jur-doc-body');

      let tree: TocTreeNode[] = [];
      if (body) {
        // Titre principal du document
        const h1 = body.querySelector<HTMLElement>(':scope > .jur-doc-h1');
        if (h1) {
          if (!h1.id) h1.id = 'jur-toc-doc-title';
          tree.push({ id: h1.id, label: (h1.textContent || '').trim(), level: 0, type: 'section', children: [] });
        }
        tree = tree.concat(buildTocTree(body, 0, null));
      }

      if (tree.length <= 1 && body) {
        const flatHeadings = Array.from(body.querySelectorAll<HTMLElement>('.jur-zone-title'));
        const seenIds = new Set<string>();
        const flatNodes = flatHeadings.map((el, idx) => {
          const label = (el.textContent || '').trim();
          if (!label) return null;
          let slug = `toc-${label
            .toLowerCase()
            .normalize('NFD').replace(/[\u0300-\u036f]/g, '')
            .replace(/[^a-z0-9]+/g, '-')
            .replace(/^-+|-+$/g, '')
            .slice(0, 40)}`;
          if (!slug || slug === 'toc-') slug = `toc-section-${idx}`;
          if (seenIds.has(slug)) slug = `${slug}-${idx}`;
          seenIds.add(slug);
          if (!el.id) el.id = slug;
          tocSectionIdsRef.current.push(el.id);
          const node: TocTreeNode = { id: el.id, label, level: 1, type: 'section', children: [] };
          return node;
        }).filter((n): n is TocTreeNode => n !== null);
        if (flatNodes.length > 0) tree = tree.concat(flatNodes);
      }

      // On masque le plan s'il n'apporte rien (titre seul ou aucune section/article).
      const countNodes = (list: TocTreeNode[]): number =>
        list.reduce((acc, n) => acc + 1 + countNodes(n.children), 0);
      const hasContent = countNodes(tree) > 1;
      if (!hasContent) tocSectionIdsRef.current = [];

      setTocTree(hasContent ? tree : []);
      setActiveTocId(tree[0]?.id ?? null);
      setExpandedTocIds(new Set());
    });

    return () => cancelAnimationFrame(raf);
  }, [viewedDoc, buildTocTree]);

  /** Bascule l'état plié/déplié d'un nœud du plan (n'affecte que l'affichage
   *  de la sidebar, jamais le contenu déjà entièrement rendu). */
  const toggleTocNode = useCallback((id: string) => {
    setExpandedTocIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  useEffect(() => {
    const container = viewerContentRef.current;
    if (!container || tocSectionIdsRef.current.length === 0) return;

    tocObserverRef.current?.disconnect();

    const observer = new IntersectionObserver(
      (observerEntries) => {
        const visible = observerEntries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible.length > 0) {
          const id = visible[0].target.id;
          setActiveTocId(id);
          revealTocPath(id);
        }
      },
      {
        root: container,
        rootMargin: '0px 0px -75% 0px',
        threshold: 0,
      }
    );

    tocSectionIdsRef.current.forEach((id) => {
      const el = container.querySelector(`#${CSS.escape(id)}`);
      if (el) observer.observe(el);
    });

    tocObserverRef.current = observer;
    return () => observer.disconnect();
  }, [tocTree, revealTocPath]);

  /** Fait défiler la visionneuse jusqu'au nœud choisi dans le plan (section
   *  ou article — fonctionne pour n'importe quel id présent dans le DOM,
   *  observé ou non par le scroll-spy). */
  const scrollToTocEntry = useCallback((id: string) => {
    const container = viewerContentRef.current;
    const el = container?.querySelector(`#${CSS.escape(id)}`) as HTMLElement | null;
    if (!container || !el) return;
    container.scrollTo({
      top: el.offsetTop - 12,
      behavior: 'smooth',
    });
    setActiveTocId(id);
  }, []);

  // ── Timeline ─────────────────────────────────────────────────────────────────
  const [showTimeline, setShowTimeline] = useState(false);
  const [timelineData, setTimelineData] = useState<TimelineVersion[]>([]);
  const [timelineLoading, setTimelineLoading] = useState(false);
  // Onglet actif du panneau timeline : 'doc' = chronologie légale, 'dossier' = docs du dossier actif
  const [timelineTab, setTimelineTab] = useState<'doc' | 'dossier'>('doc');

  const loadTimeline = useCallback(async () => {
    if (!viewedDoc) return;
    setTimelineLoading(true);
    setShowTimeline(true);
    try {
      const raw = viewedDoc.raw as Record<string, unknown> | undefined;

      // ── Résolution du textCid ─────────────────────────────────────────────
      // Pour un article (LEGIARTI/JORFARTI) on cherche le textCid dans les
      // métadonnées brutes. Pour un texte (LEGITEXT/JORFTEXT/KALICONT…) c'est
      // l'id lui-même.
      const isArticle = /^(LEGIARTI|JORFARTI|KALIARTI)/.test(viewedDoc.id);
      const textCid: string = isArticle
        ? ((raw?.textCid || raw?.cidTexte || raw?.idTexte || raw?.cid || '') as string) || viewedDoc.id
        : viewedDoc.id;

      // Si on n'a toujours qu'un id d'article comme textCid, la requête
      // échouera côté API — on signale le problème proprement.
      if (isArticle && /^(LEGIARTI|JORFARTI|KALIARTI)/.test(textCid)) {
        console.warn('[Timeline] textCid non résolu pour', viewedDoc.id, '— raw:', raw);
        setTimelineData([]);
        setTimelineLoading(false);
        return;
      }

      const year = new Date().getFullYear();
      const res = await fetch(`${API_BASE}/api/juridique/chrono-text`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          text_cid: textCid,
          date_consult: new Date().toISOString().slice(0, 10),
          start_year: year - 50,
          end_year: year,
        }),
      });
      const data = await res.json();

      // Normalisation de la réponse ChronolegiResponse

      const versions: TimelineVersion[] = [];

      const regroupements: Record<string, unknown>[] = Array.isArray(data.regroupements)
        ? data.regroupements
        : [];

      for (const regroupement of regroupements) {
        const versionMap = regroupement.versions as Record<string, Record<string, unknown>> | undefined;
        if (!versionMap) continue;

        for (const [dateKey, v] of Object.entries(versionMap)) {
          // Collecter les articles modificateurs pour afficher des infos utiles
          const mods: Array<{ id: string; title: string; action: string }> = [];
          const articlesModificateurs = v.articlesModificateurs as Record<string, Record<string, unknown>> | undefined;
          if (articlesModificateurs) {
            for (const [, artMod] of Object.entries(articlesModificateurs)) {
              const actions = artMod.actions as Record<string, Record<string, unknown>> | undefined;
              const firstAction = actions ? Object.keys(actions)[0] : '';
              mods.push({
                id: (artMod.id || '') as string,
                title: (artMod.title || '') as string,
                action: firstAction,
              });
            }
          }

          // dateDebut est un timestamp ms sous forme de string
          const dateDebutRaw = (v.dateDebut || dateKey) as string;
          const dateFinRaw = (v.dateFin || '') as string;

          versions.push({
            id: dateKey,
            dateDebut: dateDebutRaw,
            dateFin: dateFinRaw || undefined,
            etat: (v.isEndVersion ? 'VIGUEUR' : 'MODIFIE') as string,
            isEndVersion: !!v.isEndVersion,
            modificateurs: mods,
          });
        }
      }

      // Tri antéchronologique → chronologique (plus ancienne en premier)
      versions.sort((a, b) => Number(a.dateDebut) - Number(b.dateDebut));

      setTimelineData(versions);
    } catch (e) {
      console.error('[Timeline] erreur:', e);
      setTimelineData([]);
    } finally {
      setTimelineLoading(false);
    }
  }, [viewedDoc]);

  // Recharge la timeline automatiquement si elle est déjà ouverte et que le document change
  useEffect(() => {
    if (showTimeline && viewedDoc?.source === 'legifrance') {
      setTimelineData([]);
      loadTimeline();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewedDoc?.id]);

  // ── Prévisualisation au survol des liens ────────────────────────────────────
  const [linkPreview, setLinkPreview] = useState<{
    id: string;
    x: number;
    y: number;
    content: string | null; // null = en cours de chargement
    error?: boolean;
  } | null>(null);
  const linkPreviewCache = useRef<Map<string, string>>(new Map());
  const linkPreviewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  /** Résout l'endpoint de preview selon le préfixe de l'identifiant. **/

  const resolvePreviewEndpoint = useCallback((id: string, cid?: string): string => {
    const base = `${API_BASE}/api/juridique`;
    if (cid) return `${base}/article-version/${encodeURIComponent(cid)}/${encodeURIComponent(id)}`;
    if (/^(LEGIARTI|JORFARTI|KALIARTI|CNILARTICLE)/.test(id))
      return `${base}/article/${encodeURIComponent(id)}`;
    if (/^(LEGITEXT|JORFTEXT)/.test(id))
      return `${base}/loda/${encodeURIComponent(id)}`;
    if (/^JURITEXT/.test(id))
      return `${base}/juri/${encodeURIComponent(id)}`;
    if (/^KALITEXT/.test(id))
      return `${base}/kali/${encodeURIComponent(id)}`;
    // Décision JUDILIBRE
    if (/^[0-9a-f]{24}$/.test(id))
      return `${base}/decision/${encodeURIComponent(id)}?resolve_references=true`;
    return `${base}/article/${encodeURIComponent(id)}`;
  }, []);

  const handleLinkMouseEnter = useCallback((e: ReactMouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const citeEl = target.closest('[data-cite-id]') as HTMLElement | null;
    if (!citeEl) return;
    const id = citeEl.dataset.citeId;
    if (!id) return;
    const cid = citeEl.dataset.citeCid;
    const cacheKey = cid ? `${cid}::${id}` : id;
    const rect = citeEl.getBoundingClientRect();
    const x = rect.left;
    const y = rect.bottom + 6;
    if (citeEl.title) {
      citeEl.dataset.savedTitle = citeEl.title;
      citeEl.removeAttribute('title');
    }
    // Parcourir les ancêtres portant un title (ex: l'<a> parent)
    let ancestor = citeEl.parentElement;
    while (ancestor && ancestor !== e.currentTarget) {
      if (ancestor.title) {
        ancestor.dataset.savedTitle = ancestor.title;
        ancestor.removeAttribute('title');
      }
      ancestor = ancestor.parentElement;
    }

    // Annuler le timer précédent (évite les flash sur survol rapide)
    if (linkPreviewTimer.current) clearTimeout(linkPreviewTimer.current);

    // Si déjà en cache, affichage immédiat
    if (linkPreviewCache.current.has(cacheKey)) {
      setLinkPreview({ id, x, y, content: linkPreviewCache.current.get(cacheKey)! });
      return;
    }

    // Afficher "chargement" après 200ms (évite la bulle si le survol est fugace)
    linkPreviewTimer.current = setTimeout(async () => {
      setLinkPreview({ id, x, y, content: null });
      try {
        const endpoint = resolvePreviewEndpoint(id, cid);
        const res = await fetch(endpoint);
        if (!res.ok) throw new Error(`${res.status}`);
        const raw = await res.json();
        const text = extractPreviewText(raw, id) || '(Contenu non disponible)';
        linkPreviewCache.current.set(cacheKey, text);
        setLinkPreview((prev) => prev?.id === id ? { ...prev, content: text } : prev);
      } catch {
        setLinkPreview((prev) => prev?.id === id ? { ...prev, content: '(Erreur de chargement)', error: true } : prev);
      }
    }, 250);
  }, [resolvePreviewEndpoint]);

  const handleLinkMouseLeave = useCallback((e: ReactMouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const citeEl = target.closest('[data-cite-id]') as HTMLElement | null;
    if (!citeEl) return;
    if (linkPreviewTimer.current) clearTimeout(linkPreviewTimer.current);
    setLinkPreview(null);
    // Restaurer les title natifs supprimés
    if (citeEl.dataset.savedTitle) {
      citeEl.setAttribute('title', citeEl.dataset.savedTitle);
      delete citeEl.dataset.savedTitle;
    }
    let ancestor = citeEl.parentElement;
    while (ancestor && ancestor !== e.currentTarget) {
      if (ancestor.dataset.savedTitle) {
        ancestor.setAttribute('title', ancestor.dataset.savedTitle);
        delete ancestor.dataset.savedTitle;
      }
      ancestor = ancestor.parentElement;
    }
  }, []);

  // ── Gestionnaire de clic sur citation ────────────────────────────────────────
  const handleCitationClick = useCallback((e: ReactMouseEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const citeEl = (target.closest('[data-cite-id]') || target) as HTMLElement;
    const id = citeEl.dataset?.citeId;
    if (!id) return;
    const cid = citeEl.dataset?.citeCid;
    e.preventDefault();
    // Fermer la bulle de prévisualisation au clic
    if (linkPreviewTimer.current) clearTimeout(linkPreviewTimer.current);
    setLinkPreview(null);
    openDocumentRef.current(cid ? { id, _versionCid: cid } : { id });
  }, []);

  // ── Export ──────────────────────────────────────────────────────────────────
  const exportMarkdown = useCallback(() => {
    if (!viewedDoc) return;
    const blob = new Blob([viewedDoc.contenu], { type: 'text/markdown' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${viewedDoc.titre.replace(/[^a-z0-9]/gi, '_').slice(0, 60)}.md`;
    a.click();
  }, [viewedDoc]);

  // ── Charger la totalité des articles cités inline ───────────────────────────
  const [loadingAllArticles, setLoadingAllArticles] = useState(false);
  const loadAllArticles = useCallback(async () => {
    const doc = viewedDoc;
    if (!doc || !doc.rawBody || doc.allArticlesLoaded) return;
    setLoadingAllArticles(true);
    try {
      const body = await resolveInlineCitations(doc.rawBody, fetchArticleHtml, Infinity);
      setViewedDoc((cur) => (cur && cur.id === doc.id)
        ? { ...cur, content: { ...cur.content, body }, contenu: htmlToPlainBlocks(body), allArticlesLoaded: true }
        : cur);
    } finally {
      setLoadingAllArticles(false);
    }
  }, [viewedDoc, fetchArticleHtml]);

  return {
    viewedDoc, setViewedDoc, loadingDoc, docError, setDocError,
    docStack, setDocStack, goBack, closeDocument, openDocument, openDossierLegislatif,
    viewerContentRef,
    tocTree, activeTocId, expandedTocIds, toggleTocNode, scrollToTocEntry,
    showTimeline, setShowTimeline, timelineData, timelineLoading, timelineTab, setTimelineTab, loadTimeline,
    linkPreview, setLinkPreview, handleLinkMouseEnter, handleLinkMouseLeave, handleCitationClick,
    exportMarkdown,
    loadingAllArticles, loadAllArticles,
  };
}
