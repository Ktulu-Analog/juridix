// ============================================================================
// Juridix — Espace "Recherche juridique" — Hook useDocumentViewer
// ============================================================================
// État et logique de la visionneuse de document : ouverture d'un document
// (openDocument, routée par fond/source vers le bon endpoint backend), pile
// de navigation (goBack/closeDocument), table des matières flottante
// (scroll-spy), timeline des versions légales, prévisualisation au survol
// des liens de citation, et export Markdown.
//
// Dépendance externe passée en paramètres — c'est le point de couplage avec
// la recherche documenté dans le plan de refactor initial : `openDocument`
// a besoin de `fond`/`source` (état courant du FORMULAIRE de recherche, pas
// resultsFond/resultsSource) comme repli pour la navigation par citation,
// indépendante d'une recherche (ex. clic sur un lien dans un document déjà
// ouvert). Les clics depuis la liste de résultats passent explicitement
// resultsFond/resultsSource en arguments d'appel (overrideFond/overrideSource)
// depuis le JSX de JuridiquePanel.tsx — ce hook n'a donc besoin de rien
// connaître de useJuridiqueSearch au-delà de ces deux valeurs.
//
// viewedDoc/setViewedDoc/docStack/setDocStack/docError/setDocError sont
// également nécessaires à useFavorites/useDossiers (déjà instanciés avec ces
// valeurs en paramètres) : useDocumentViewer doit donc être appelé AVANT eux
// dans le composant. viewerContentRef est de même exposée pour être transmise
// à useAnnotations (scroll-vers-annotation dans le même conteneur que le
// scroll-spy de la TOC).
//
// Extrait de JuridiquePanel.tsx (étape 5 du refactor — cf. discussion du
// 05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { useState, useCallback, useRef, useEffect, type MouseEvent as ReactMouseEvent } from 'react';
import { API_BASE } from '../../../constants';
import {
  isTextId, stripHtml, resolveInlineCitations, countCitableArticleIds, DEFAULT_MAX_INLINE_CITATIONS,
} from '../content/html-helpers';
import { extractTextContent } from '../content';
import { extractArticleInlineText } from '../content/extract-loda';
import { extractTitle, extractResultId } from '../search-helpers';
import type {
  SearchResult, Source, Fond, DocType, ViewedDoc, TocTreeNode, TimelineVersion, DossierLegislatifRef,
} from '../types';

// Mapping fond → endpoint backend pour les fonds spéciaux (listes)
function fondToEndpointInfo(fond: Fond, id: string, idIsTextId: boolean): { endpoint: string; type: DocType } {
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

    // NB : ces 3 branches (BOCC / QUESTION_ECRITE / DOC_ADMIN) ne sont plus
    // atteintes par `openDocument`, qui ouvre désormais ces fonds directement
    // depuis l'objet résultat déjà en mémoire — l'API officielle n'expose
    // aucun endpoint de consultation à l'unité pour eux (voir le commentaire
    // dans `openDocument`). Conservées ici par défense uniquement.
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

  // ── Résolution inline des citations d'article (cf. resolveInlineCitations
  // dans html-helpers.ts) : va chercher le texte de chaque article cité pour
  // l'insérer directement dans le document affiché, plutôt que de dépendre
  // d'un survol de souris — le texte doit être présent dans le HTML dès le
  // rendu pour qu'un LLM (bouton « Analyser avec l'IA », export de dossier…)
  // puisse le voir. Cache en mémoire (Map, durée de vie du composant) pour
  // éviter de re-fetcher un même article cité plusieurs fois dans la même
  // session — les renvois vers un article socle (ex. l'article 5 modifié par
  // plusieurs décrets successifs) reviennent souvent d'un document à l'autre.
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
    // Fond/source à utiliser pour router l'ouverture de CE document. Par
    // défaut, on retombe sur le fond/source *courants* (utile pour la
    // navigation par lien de citation, indépendante d'une recherche). Les
    // clics depuis la liste de résultats passent explicitement
    // resultsFond/resultsSource pour garantir que le document ouvert
    // corresponde bien au fond qui a produit ce résultat, même si le
    // filtre a changé depuis la recherche.
    overrideFond?: Fond,
    overrideSource?: Source,
  ) => {
    // Fond/source effectifs pour CETTE ouverture : priorité aux valeurs
    // explicitement fournies par l'appelant (liste de résultats), sinon
    // repli sur le filtre courant (navigation par lien de citation).
    //
    // ATTENTION : pour la navigation par lien de citation (pas d'override),
    // ce repli sur `source` — le filtre du FORMULAIRE de recherche — est
    // incorrect dès que le document actuellement ouvert n'est pas dans le
    // même fond que ce filtre (ex. une décision JUDILIBRE ouverte pendant
    // que l'onglet de recherche est resté sur « Légifrance » : cas courant
    // depuis que le panneau de résultats et la visionneuse sont
    // indépendants). Un id JUDILIBRE (hash Mongo, 24 caractères hex) ne
    // correspond à aucun préfixe Légifrance connu de `fondToEndpointInfo` :
    // sans détection par motif, la requête part vers `/article/{id}` et
    // échoue silencieusement. On détecte donc `judilibre` par la FORME de
    // l'id quand aucune source n'est explicitement fournie — c'est
    // exactement le motif déjà utilisé par `resolvePreviewEndpoint` pour la
    // bulle de prévisualisation au survol, qui elle ne souffre pas de ce bug
    // car elle ne dépend jamais de `source`/`fond`.
    const isLikelyJudilibreId = /^[0-9a-f]{24}$/i.test(result.id || '');
    // Même bug que celui corrigé côté Judilibre ci-dessus, mais entre fonds
    // Légifrance eux-mêmes : `fondToEndpointInfo` route CODE_*/LODA_*/JORF
    // par le FILTRE de recherche courant plutôt que par la forme de l'id
    // pour les textes (ex. LODA_ETAT + clic sur un lien vers un JORFTEXT
    // route à tort vers /loda/ au lieu de /jorf-part/, /consult/legiPart
    // n'étant pas fait pour un id JORFTEXT). Ça ne posait pas de problème
    // tant qu'aucun lien de citation ne traversait les fonds — exactement
    // ce que A2 introduit avec `LienModification.textCid`
    // (LienModification pointe vers le texte modificateur, qui peut être un
    // JORFTEXT même depuis un article LODA/CODE). Repli sur le routage par
    // préfixe d'id (comme `resolvePreviewEndpoint`, déjà immunisé) dès qu'il
    // n'y a pas d'override explicite, càd dès qu'on est en navigation par
    // citation plutôt qu'en clic depuis la liste de résultats.
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

      // Lien de version d'article (cf. renderArticleVersionsBlock) : on
      // route directement vers l'endpoint dédié /article-version/:cid/:id
      // plutôt que par le routage habituel par fond, car /consult/
      // getArticle ne renvoie pas fiablement le contenu d'une version
      // historique précise.
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
        // Résolution inline des citations (cf. commentaire sur
        // fetchArticleHtml) — insère le texte des articles cités
        // directement dans le HTML avant de dériver contenuLlm.
        const totalInlineArticles = countCitableArticleIds(rawContent.body);
        const content = { ...rawContent, body: await resolveInlineCitations(rawContent.body, fetchArticleHtml) };
        const contenuLlm = stripHtml(content.body);
        setViewedDoc({
          id, titre, content, contenu: contenuLlm, source: effSource, type: 'article', raw,
          rawBody: totalInlineArticles > 0 ? rawContent.body : undefined,
          totalInlineArticles,
          allArticlesLoaded: totalInlineArticles <= DEFAULT_MAX_INLINE_CITATIONS,
        });
        setLoadingDoc(false);
        return;
      }

      // BOCC / QUESTION_ECRITE / DOC_ADMIN : l'API officielle n'expose AUCUN
      // endpoint de consultation à l'unité pour ces 3 fonds (confirmé par le
      // swagger : ni `/consult/*` ni paramètre `id` sur leurs `/list/*`).
      // L'objet résultat, déjà récupéré et normalisé lors de la recherche,
      // contient déjà l'intégralité des champs disponibles (titre, dates,
      // chemin du PDF...) : on l'ouvre directement, sans appel réseau
      // superflu — et surtout sans rappeler par erreur l'endpoint de liste
      // (ce qui produisait auparavant la liste entière au lieu du document).
      //
      // DEBAT : contournement temporaire. `/consult/debat` existe bel et
      // bien sur le papier (swagger), mais PISTE renvoie en pratique une
      // 400 "Le nombre de résultat retourné par le moteur de recherche
      // n'est pas égal à 1" pour une partie des `id` renvoyés par
      // `/list/debatsParlementaires` — désynchronisation entre l'index de
      // liste et l'index de consultation côté DILA, hors de notre contrôle.
      // L'objet résultat de liste contient déjà tout ce qu'on affiche
      // (métadonnées + lien PDF via attachmentUrl/pathToFile, cf.
      // extractTextContent) : on l'ouvre directement, comme pour les 3
      // fonds ci-dessus, et on évite l'appel `/consult/debat` en attendant
      // un correctif DILA. À retirer si/quand PISTE corrige l'index.
      if (effSource === 'legifrance' && (effFond === 'BOCC' || effFond === 'QUESTION_ECRITE' || effFond === 'DOC_ADMIN' || effFond === 'DEBAT')) {
        const type: ViewedDoc['type'] = effFond === 'BOCC' ? 'bocc' : effFond === 'QUESTION_ECRITE' ? 'question_ecrite' : effFond === 'DEBAT' ? 'debat' : 'doc_admin';
        const titre = extractTitle(result, effSource);
        const content = extractTextContent(result, type, titre);
        const contenuLlm = stripHtml(content.body);
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
      // Résolution inline des citations (cf. fetchArticleHtml ci-dessus) —
      // no-op immédiat si le HTML ne contient aucun data-cite-id (c'est le
      // cas des décisions Judilibre, en format 'markdown').
      const totalInlineArticles = countCitableArticleIds(rawContent.body);
      const content = { ...rawContent, body: await resolveInlineCitations(rawContent.body, fetchArticleHtml) };
      // Texte brut pour LLM = HTML strippé
      const contenuLlm = stripHtml(content.body);

      setViewedDoc({
        id, titre, content, contenu: contenuLlm, source: effSource, type, raw,
        rawBody: totalInlineArticles > 0 ? rawContent.body : undefined,
        totalInlineArticles,
        allArticlesLoaded: totalInlineArticles <= DEFAULT_MAX_INLINE_CITATIONS,
        // A4 — le champ est présent à la fois sur `raw` (réponse de
        // consultation du texte, ConsultTextResponse/ConsultJorfResponse) ET
        // sur le SearchResult ayant servi à l'ouvrir (cf. swagger officiel
        // Légifrance v2.4.2) — on privilégie `raw` (plus à jour / présent
        // même hors recherche, ex. citation directe), repli sur le résultat
        // de recherche sinon.
        dossiersLegislatifs: (raw as { dossiersLegislatifs?: DossierLegislatifRef[] })?.dossiersLegislatifs
          ?? result.dossiersLegislatifs,
      });
    } catch (e: unknown) {
      setDocError((e as Error).message || 'Erreur de chargement');
    } finally {
      setLoadingDoc(false);
    }
  }, [source, fond, fetchArticleHtml]);

  // ── A4 : ouverture d'un dossier législatif ──────────────────────────────────
  // Contrairement à `openDocument`, pas de routage par fond : l'appelant
  // (carte de résultat LODA/JORF ou bouton dans l'en-tête de la visionneuse,
  // cf. JuridiquePanel.tsx) fournit directement l'id `JORFDOLE…` extrait de
  // `SearchResult.dossiersLegislatifs`/`ViewedDoc.dossiersLegislatifs`.
  // Endpoint déjà exposé côté backend (GET /dossier/:id, cf.
  // routes.rs::get_dossier) — cf. réserves sur le schéma de réponse dans
  // content/extract-dossier-legislatif.ts.
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
      const contenuLlm = stripHtml(content.body);
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
  // Nœuds actuellement dépliés dans la sidebar (id de section). Purement
  // affichage : ne modifie jamais le contenu déjà entièrement rendu dans la
  // visionneuse (tout le texte du code/texte est présent dans le DOM dès
  // l'ouverture — seul le PLAN de navigation se replie/déplie).
  const [expandedTocIds, setExpandedTocIds] = useState<Set<string>>(new Set());
  const tocObserverRef = useRef<IntersectionObserver | null>(null);
  // id de nœud → id de son parent direct, reconstruit à chaque build de
  // l'arbre — permet de remonter la chaîne d'ancêtres à déplier quand le
  // scroll-spy active un nœud profondément imbriqué (cf. revealTocPath).
  const tocParentRef = useRef<Map<string, string>>(new Map());
  // ids observés par le scroll-spy : uniquement les SECTIONS (Livre/Titre/
  // Chapitre/Section), jamais les articles pris individuellement — un gros
  // code (Code civil, Code du travail…) compte plusieurs milliers d'articles,
  // les observer tous alourdirait l'IntersectionObserver pour un gain nul
  // (la granularité "section courante" suffit à la lecture). La navigation
  // vers un article précis reste possible via un clic dans le plan déplié
  // (scrollToTocEntry fonctionne pour n'importe quel id, observé ou non).
  const tocSectionIdsRef = useRef<string[]>([]);

  /** Ajoute à l'ensemble des nœuds dépliés toute la chaîne d'ancêtres d'un id
   *  donné, pour que la sidebar révèle automatiquement la position de lecture
   *  courante (comportement Dalloz) — sans jamais replier ce que l'utilisateur
   *  a déjà ouvert manuellement (on ajoute, on ne retire jamais ici). */
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

  // Construction récursive de l'arbre à partir du DOM rendu : chaque
  // `<section class="jur-section">` produit par renderSections (id stable
  // désormais assigné côté rendu, cf. content/html-helpers.ts) devient un
  // nœud, ses enfants directs `<article class="jur-article">` et `<section>`
  // imbriquées sont parcourus récursivement — la profondeur réelle du texte
  // (Livre > Titre > Chapitre > Section > Article) est ainsi préservée,
  // contrairement à l'ancien scan à plat qui aplatissait tout en un seul
  // niveau indistinct dès que la profondeur dépassait le titre du document.
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

    // Le HTML est injecté via dangerouslySetInnerHTML — on laisse le DOM se
    // peindre avant de scanner (sinon querySelectorAll renvoie l'ancien arbre).
    const raf = requestAnimationFrame(() => {
      tocParentRef.current = new Map();
      tocSectionIdsRef.current = [];
      const body = container.querySelector<HTMLElement>('.jur-doc-body');

      let tree: TocTreeNode[] = [];
      if (body) {
        // Titre principal du document : conservé comme premier repère du
        // plan (comme avant), mais SANS devenir le parent du reste de
        // l'arbre — la hiérarchie réelle (Livre, Titre…) démarre elle aussi
        // au niveau 0 juste en dessous.
        const h1 = body.querySelector<HTMLElement>(':scope > .jur-doc-h1');
        if (h1) {
          if (!h1.id) h1.id = 'jur-toc-doc-title';
          tree.push({ id: h1.id, label: (h1.textContent || '').trim(), level: 0, type: 'section', children: [] });
        }
        tree = tree.concat(buildTocTree(body, 0, null));
      }

      // Repli : documents sans structure sections/articles imbriquée (ex.
      // décisions Judilibre, qui utilisent des zones à plat `.jur-zone-title`
      // plutôt que renderSections) — liste plate à un seul niveau, comme
      // avant le passage au plan arborescent ci-dessus.
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

  // Scroll-spy : observe les sections (cf. tocSectionIdsRef) et active celle
  // la plus proche du haut du viewport de lecture (signature Doctrine : le
  // plan suit la lecture sans action de l'utilisateur), en révélant
  // automatiquement son chemin dans la sidebar.
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
        // Fenêtre d'activation resserrée en haut du viewer pour un scroll-spy
        // précis, façon "ligne de lecture" plutôt que "élément visible".
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

      // ── Normalisation de la réponse ChronolegiResponse ────────────────────
      // Structure réelle : { regroupements: RegroupementDTO[] }
      // Chaque RegroupementDTO : { title: string, versions: { [date]: VersionDTO } }
      // Chaque VersionDTO : { dateDebut: string (ts ms), isEndVersion: boolean, articlesModificateurs: {...} }
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

  /** Résout l'endpoint de preview selon le préfixe de l'identifiant.
   *  Si `cid` est fourni (lien de version d'article), on route vers
   *  l'endpoint dédié qui va chercher la bonne version via getArticleByCid —
   *  /consult/getArticle avec l'id d'une version historique ne renvoie pas
   *  fiablement son contenu (cf. renderArticleVersionsBlock). */
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
    // Clé de cache distincte des liens de version (même id de version =
    // toujours le même contenu, mais on inclut le cid par prudence/lisibilité).
    const cacheKey = cid ? `${cid}::${id}` : id;

    // Position de la bulle : juste sous l'élément survolé
    const rect = citeEl.getBoundingClientRect();
    // Coordonnées fixes dans le viewport (position: fixed sur la bulle)
    const x = rect.left;
    const y = rect.bottom + 6;

    // Masquer le title natif du navigateur pour éviter le double tooltip
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
  // Relance `resolveInlineCitations` sur le HTML brut (pré-résolution) du
  // document courant, sans plafond (`maxLinks = Infinity`), pour les textes
  // qui citent plus d'articles que `DEFAULT_MAX_INLINE_CITATIONS` (cf.
  // bouton « Charger tous les articles » à côté de « Comparer » dans
  // JuridiquePanel.tsx). Session-only : `rawBody` n'est pas persisté, donc
  // sans effet après réouverture du document.
  const [loadingAllArticles, setLoadingAllArticles] = useState(false);
  const loadAllArticles = useCallback(async () => {
    const doc = viewedDoc;
    if (!doc || !doc.rawBody || doc.allArticlesLoaded) return;
    setLoadingAllArticles(true);
    try {
      const body = await resolveInlineCitations(doc.rawBody, fetchArticleHtml, Infinity);
      setViewedDoc((cur) => (cur && cur.id === doc.id)
        ? { ...cur, content: { ...cur.content, body }, contenu: stripHtml(body), allArticlesLoaded: true }
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
