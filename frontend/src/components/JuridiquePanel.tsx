// ============================================================================
// Juridix — Espace "Recherche juridique"
// ============================================================================
// Auteur  : Pierre COUGET
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================
// Trois zones :
//   ① Colonne gauche  — formulaire de recherche + liste de résultats paginée
//   ② Colonne centrale — visionneuse de document (article / décision)
//   ③ Barre d'action   — "Résumer" (auto) · "Analyser avec l'IA" (manuel)
//
// Les appels LLM ne se font PAS ici : on prépare le message via
// POST /api/juridique/envoyer-chat puis on rappelle onSendToChat().
// ============================================================================

import {
  useState,
  useRef,
  useEffect,
} from 'react';
import './JuridiquePanel.css';
import {
  sanitizeHtml, stripHtml, judilibreDisplayRef,
} from './juridique/content/html-helpers';
import type {
  AnnotationColor, ViewedDoc, JuridiquePanelProps, Source, Fond, TocTreeNode,
} from './juridique/types';
import {
  FONDS_LEGIFRANCE, SORTS_LEGIFRANCE, SORTS_JUDILIBRE, SORTS_BY_FOND,
  NATURES_LEGIFRANCE, LEGAL_STATUS_LEGIFRANCE, FIELDS_JUDILIBRE,
  OPERATEURS_JUDILIBRE, FILE_TYPES_JUDILIBRE, TYPES_PARLEMENT,
  DECISION_TYPES_FALLBACK, FONDS_SANS_RECHERCHE_TEXTUELLE,
  FONDS_AVEC_NATURE, FONDS_AVEC_ETAT_JURIDIQUE, CHAMPS_PAR_FOND,
  FOND_LABELS, DOC_TYPE_LABELS, PUB_LABELS,
} from './juridique/constants';
import {
  fondRequiresQuery, extractTitle, extractResultId, extractDate,
  extractLegifranceHighlight, solutionModifier,
} from './juridique/search-helpers';
import { useFavorites } from './juridique/hooks/useFavorites';
import { useAnnotations } from './juridique/hooks/useAnnotations';
import { useDossiers } from './juridique/hooks/useDossiers';
import { useJuridiqueSearch } from './juridique/hooks/useJuridiqueSearch';
import { useDocumentViewer, parseChronoDate } from './juridique/hooks/useDocumentViewer';

// NB : React/useCallback/useMemo/ReactMarkdown/remarkGfm/API_BASE/
// ARTIFACT_MD_COMPONENTS et une bonne partie de html-helpers/storage/content
// étaient importés ici avant le découpage en hooks (étapes 1-4 du refactor)
// mais ne sont plus utilisés directement dans ce fichier — leur usage a migré
// vers les hooks dédiés. Nettoyé pour éviter la confusion sur qui utilise
// quoi. `ReactMarkdown`/`remarkGfm`/`ARTIFACT_MD_COMPONENTS` étaient déjà du
// code mort AVANT le refactor : `ContentFormat` prévoit un mode 'markdown',
// mais aucun extracteur ne le produit (tous renvoient 'html') et le rendu
// (plus bas) passe toujours par dangerouslySetInnerHTML — jamais par
// ReactMarkdown. À garder en tête si vous réintroduisez un jour du contenu
// Markdown pur.

// ── Types et constantes ──────────────────────────────────────────────────────
// Déplacés vers ./juridique/types.ts et ./juridique/constants.ts
// (étape 3 du refactor — cf. discussion du 04-05/07/2026).

// Les helpers purs de recherche/résultats (fondRequiresQuery, extractTitle,
// extractResultId, extractDate, extractLegifranceHighlight, solutionModifier,
// normalizeBoccTextResult, normalizeDebatResult, normalizeQuestionResult,
// normalizeDocAdminResult) vivent désormais dans ./juridique/search-helpers,
// partagés entre useJuridiqueSearch, useDocumentViewer et le JSX resté ici.
// fondToEndpointInfo vit désormais dans ./juridique/hooks/useDocumentViewer
// (seul consommateur).

// Libellés/icônes des modes d'analyse de dossier — partagés entre les
// boutons de lancement et l'historique des analyses passées.
const DOSSIER_LLM_MODE_LABELS: Record<'synthese' | 'contradictoire' | 'points_cles', { icon: string; label: string }> = {
  synthese: { icon: '📋', label: 'Synthèse' },
  contradictoire: { icon: '⚔️', label: 'Contradictoire' },
  points_cles: { icon: '🎯', label: 'Points clés' },
};

// ── Composant principal ───────────────────────────────────────────────────────

/** Rendu récursif du plan interactif (sommaire façon Dalloz) : chaque nœud
 *  section/article de `TocTreeNode` devient une entrée éventuellement
 *  dépliable (chevron) — seules les sections avec enfants portent un
 *  chevron, un article est toujours une feuille cliquable. Composant garde
 *  volontairement en dehors de JuridiquePanel (pas de dépendance à son
 *  state) : il ne fait que refléter les props reçues, tout l'état
 *  (nœuds dépliés, nœud actif) reste piloté par useDocumentViewer. */
function TocTreeList({
  nodes, activeId, expandedIds, onToggle, onSelect,
}: {
  nodes: TocTreeNode[];
  activeId: string | null;
  expandedIds: Set<string>;
  onToggle: (id: string) => void;
  onSelect: (id: string) => void;
}) {
  return (
    <ul className="jur-toc-list">
      {nodes.map((node) => {
        const hasChildren = node.children.length > 0;
        const expanded = expandedIds.has(node.id);
        return (
          <li key={node.id} className="jur-toc-node">
            <div
              className="jur-toc-row"
              data-toc-node-id={node.id}
              style={{ paddingLeft: Math.min(node.level, 5) * 12 }}
            >
              {hasChildren ? (
                <button
                  className="jur-toc-caret"
                  onClick={() => onToggle(node.id)}
                  title={expanded ? 'Replier' : 'Déplier'}
                  aria-label={expanded ? 'Replier' : 'Déplier'}
                >
                  {expanded ? '▾' : '▸'}
                </button>
              ) : (
                <span className="jur-toc-caret jur-toc-caret--spacer" aria-hidden="true" />
              )}
              <button
                className={`jur-toc-item jur-toc-item--${node.type}${node.level === 0 ? ' jur-toc-item--level-0' : ''}${activeId === node.id ? ' jur-toc-item--active' : ''}`}
                onClick={() => onSelect(node.id)}
                title={node.label}
              >
                {node.label}
              </button>
            </div>
            {hasChildren && expanded && (
              <TocTreeList
                nodes={node.children}
                activeId={activeId}
                expandedIds={expandedIds}
                onToggle={onToggle}
                onSelect={onSelect}
              />
            )}
          </li>
        );
      })}
    </ul>
  );
}

export function JuridiquePanel({ onSendToChat, onShowInChat, onClose, globalControls }: JuridiquePanelProps) {
  // ── Panneau de facettes : repliable, masqué par défaut, posé en overlay
  // au-dessus du corps (façon Dalloz/Lexis) — plus une simple variante mobile.
  // Déclaré avant useJuridiqueSearch() car passé en paramètre (conditionne le
  // préchargement des taxonomies JUDILIBRE). ─────────────────────────────────
  const [facetsOpen, setFacetsOpen] = useState(true);
  const [openFacets, setOpenFacets] = useState<Record<string, boolean>>({
    fond: true, dates: true, tri: false, champ: false, jurisdiction: true, publication: false, solution: false,
    nature: false, legalStatus: false, cnil: false,
    chamber: false, formation: false, theme: false, location: false,
    judField: false, fileType: false, specialFilters: true,
  });
  const toggleFacet = (key: string) =>
    setOpenFacets((prev) => ({ ...prev, [key]: !prev[key] }));

  // ── Recherche / filtres / pagination ────────────────────────────────────────
  const {
    source, setSource, fond, setFond, query, setQuery,
    champRecherche, setChampRecherche, dateStart, setDateStart, dateEnd, setDateEnd,
    sort, setSort, jurisdiction, setJurisdiction, publication, setPublication,
    solution, setSolution, nature, setNature, legalStatus, setLegalStatus,
    chamber, setChamber, formation, setFormation, theme, setTheme, themeInput, setThemeInput,
    location, setLocation, locationInput, setLocationInput,
    judField, setJudField, judOperator, setJudOperator,
    withFileOfType, setWithFileOfType, particularInterest, setParticularInterest,
    boccIdcc, setBoccIdcc, typesPublication, setTypesPublication,
    parlementTypes, setParlementTypes, docAdminYears, setDocAdminYears,
    cnilType, setCnilType, cnilTypeInput, setCnilTypeInput,
    cnilNatureDelib, setCnilNatureDelib, cnilNatureDelibInput, setCnilNatureDelibInput,
    page, setPage, PAGE_SIZE,
    results, setResults, resultsFond, resultsSource, total, setTotal, loading, error, hasSearched,
    history,
    taxonomiesLoading, fetchTaxonomy, taxoContext,
    chamberOptions, formationOptions, themeOptions, locationOptions,
    decisionTypeOptions, jurisdictionOptions, publicationOptions, solutionOptions,
    doSearch, handleSubmit, resetFilters, activeFilterCount, totalPages, searchFor,
  } = useJuridiqueSearch(facetsOpen);

  // ── Visionneuse de document (ouverture, pile de navigation, TOC,
  // timeline, prévisualisation de citation, export) ───────────────────────────
  // Reçoit fond/source courants comme repli pour openDocument (navigation par
  // lien de citation, indépendante d'une recherche) — cf. commentaire dans
  // useDocumentViewer.ts.
  const {
    viewedDoc, setViewedDoc, loadingDoc, docError, setDocError,
    docStack, setDocStack, goBack, closeDocument, openDocument, openDossierLegislatif,
    viewerContentRef,
    tocTree, activeTocId, expandedTocIds, toggleTocNode, scrollToTocEntry,
    showTimeline, setShowTimeline, timelineData, timelineLoading, timelineTab, setTimelineTab, loadTimeline,
    linkPreview, setLinkPreview, handleLinkMouseEnter, handleLinkMouseLeave, handleCitationClick,
    exportMarkdown,
    loadingAllArticles, loadAllArticles,
  } = useDocumentViewer(fond, source);

  // ── Favoris ──────────────────────────────────────────────────────────────────
  const {
    favorites, showFavorites, setShowFavorites, toggleFavorite, isFavorite,
  } = useFavorites(viewedDoc);

  // ── Dossier / cartable ──────────────────────────────────────────────────────
  const {
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
  } = useDossiers(viewedDoc, setViewedDoc, setDocStack, setDocError, onSendToChat, onShowInChat);

  // ── Annotations ─────────────────────────────────────────────────────────────
  // viewerContentRef vient de useDocumentViewer : partagée avec le mécanisme
  // de TOC (scroll-spy), interne à ce hook, en plus d'être utilisée ici.
  const {
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
  } = useAnnotations(viewedDoc, viewerContentRef);

  // ── Repli visuel de la TOC (purement UI, indépendant du scroll-spy) ─────────
  const [tocCollapsed, setTocCollapsed] = useState(false);
  // Garde l'entrée active visible dans la sidebar du plan quand le scroll-spy
  // la change (lecture qui avance) — sans ça, une fois le plan déplié
  // profondément, l'entrée surlignée peut sortir de la zone visible de la
  // nav sans que l'utilisateur ne fasse défiler la sidebar lui-même.
  const tocNavRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!activeTocId || tocCollapsed) return;
    const nav = tocNavRef.current;
    const el = nav?.querySelector(`[data-toc-node-id="${CSS.escape(activeTocId)}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [activeTocId, tocCollapsed]);

  // ── Citations détectées (bascule d'affichage, purement UI) ──────────────────
  const [highlightCitations, setHighlightCitations] = useState(true);
  // ── Historique des analyses LLM du dossier (repliable, purement UI) ────────
  const [showAnalysesHistory, setShowAnalysesHistory] = useState(false);

  // ── Raccourci clavier Échap : niveau lecture → niveau recherche.
  // N'intercepte pas si un champ de saisie a le focus (recherche, filtres,
  // notes…) pour ne pas gêner la frappe, et referme d'abord les
  // overlays/modales (favoris, dossier, facettes, prévisualisation de lien)
  // avant de fermer le document lui-même. ─────────────────────────────────────
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      const target = e.target as HTMLElement | null;
      const isEditable = target && (
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      );
      if (isEditable) return;
      if (linkPreview) { setLinkPreview(null); return; }
      if (showFavorites) { setShowFavorites(false); return; }
      if (showDossier) { setShowDossier(false); return; }
      if (viewedDoc) closeDocument();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [viewedDoc, closeDocument, linkPreview, showFavorites, showDossier]);

  // ── LLM ────────────────────────────────────────────────────────────────────

  const queryInputRef = useRef<HTMLInputElement>(null);

  // Focus automatique
  useEffect(() => {
    queryInputRef.current?.focus();
  }, []);

  // ── Rendu ───────────────────────────────────────────────────────────────────


  // ── Sous-composant : panneau de facettes (colonne 1, façon Lexis) ──────────
  const facetsPanel = (
    <div className={`jur-facets${facetsOpen ? '' : ' jur-facets--closed'}`}>
      <div className="jur-facets-close">
        <span className="jur-facets-close-title">Filtres</span>
        <button className="jur-btn-icon jur-btn-icon--xs" title="Fermer les filtres" onClick={() => setFacetsOpen(false)}>
          <span className="jur-burger-icon" aria-hidden="true"><span />
          <span /><span /></span>
        </button>
      </div>
      <div className="jur-source-tabs">
        <button
          className={`jur-source-tab${source === 'legifrance' ? ' jur-source-tab--active' : ''}`}
          onClick={() => {
            setSource('legifrance'); setResults([]); setPage(1); setTotal(null);
            setPublication([]); setSolution([]); setJurisdiction([]); setChampRecherche('ALL');
            setChamber([]); setFormation([]); setTheme([]); setThemeInput('');
            setLocation([]); setLocationInput(''); setJudField([]); setJudOperator('or');
            setWithFileOfType([]); setParticularInterest(false);
            // `nature` est un champ à double usage côté backend (nature de
            // texte Légifrance / nature de décision Judilibre) : on le
            // réinitialise ici aussi pour éviter qu'une valeur Judilibre
            // résiduelle soit envoyée à tort comme filtre NATURE Légifrance.
            setNature([]);
          }}
        >
          Légifrance
        </button>
        <button
          className={`jur-source-tab${source === 'judilibre' ? ' jur-source-tab--active' : ''}`}
          onClick={() => {
            setSource('judilibre'); setResults([]); setPage(1); setTotal(null);
            setNature([]); setLegalStatus([]);
            setCnilType([]); setCnilNatureDelib([]);
            setBoccIdcc(''); setTypesPublication([]); setParlementTypes([]); setDocAdminYears('');
          }}
        >
          JUDILIBRE
        </button>
      </div>

      <div className="jur-facets-title">
        <span>Filtres</span>
        {activeFilterCount > 0 && <span className="jur-facet-count">{activeFilterCount}</span>}
      </div>

      {/* Fond documentaire (Légifrance uniquement) */}
      {source === 'legifrance' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('fond')} type="button">
            <span className="jur-facet-label">Fonds documentaire</span>
            <span className={`jur-facet-chevron${openFacets.fond ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.fond && (
            <div className="jur-facet-body">
              {FONDS_LEGIFRANCE.map((f) => (
                <label
                  key={f.value}
                  className={`jur-facet-option${fond === f.value ? ' jur-facet-option--active' : ''}`}
                >
                  <input
                    type="radio"
                    name="jur-fond"
                    checked={fond === f.value}
                    onChange={() => {
                      setFond(f.value);
                      setChampRecherche('ALL');
                      // Les facettes nature/état juridique/IDCC/publication
                      // parlementaire/années n'ont de sens que pour certains
                      // fonds : on les réinitialise à chaque changement pour
                      // éviter d'envoyer un filtre incohérent avec le fond.
                      setNature([]);
                      setLegalStatus([]);
                      setBoccIdcc('');
                      setTypesPublication([]);
                      setParlementTypes([]);
                      setDocAdminYears('');
                      setCnilType([]);
                      setCnilNatureDelib([]);
                      // Le tri sélectionné peut ne plus être valide pour le
                      // nouveau fonds (cf. SORTS_BY_FOND) : on repart de
                      // Pertinence plutôt que de garder un tri incohérent.
                      setSort('PERTINENCE');
                    }}
                  />
                  <span className="jur-facet-option-label">{f.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Champ de recherche (Légifrance uniquement, selon fond) */}
      {source === 'legifrance' && (CHAMPS_PAR_FOND[fond] || CHAMPS_PAR_FOND['ALL']).length > 2 && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('champ')} type="button">
            <span className="jur-facet-label">
              Champ de recherche
              {champRecherche !== 'ALL' && <span className="jur-facet-count">●</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.champ ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.champ && (
            <div className="jur-facet-body">
              <select
                className="jur-facet-sort"
                value={champRecherche}
                onChange={(e) => setChampRecherche(e.target.value)}
              >
                {(CHAMPS_PAR_FOND[fond] || CHAMPS_PAR_FOND['ALL']).map((c) => (
                  <option key={c.value} value={c.value}>{c.label}</option>
                ))}
              </select>
            </div>
          )}
        </div>
      )}

      {/* Nature de texte (Légifrance — JORF / LODA uniquement) */}
      {source === 'legifrance' && FONDS_AVEC_NATURE.includes(fond) && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('nature')} type="button">
            <span className="jur-facet-label">
              Nature
              {nature.length > 0 && <span className="jur-facet-count">{nature.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.nature ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.nature && (
            <div className="jur-facet-body">
              {NATURES_LEGIFRANCE.map((n) => (
                <label key={n.value} className={`jur-facet-option${nature.includes(n.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={nature.includes(n.value)}
                    onChange={(e) => setNature((prev) => e.target.checked ? [...prev, n.value] : prev.filter((x) => x !== n.value))}
                  />
                  <span className="jur-facet-option-label">{n.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* État juridique (Légifrance — LODA_ETAT / CODE_ETAT / KALI uniquement) */}
      {source === 'legifrance' && FONDS_AVEC_ETAT_JURIDIQUE.includes(fond) && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('legalStatus')} type="button">
            <span className="jur-facet-label">
              État juridique
              {legalStatus.length > 0 && <span className="jur-facet-count">{legalStatus.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.legalStatus ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.legalStatus && (
            <div className="jur-facet-body">
              {LEGAL_STATUS_LEGIFRANCE.map((s) => (
                <label key={s.value} className={`jur-facet-option${legalStatus.includes(s.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={legalStatus.includes(s.value)}
                    onChange={(e) => setLegalStatus((prev) => e.target.checked ? [...prev, s.value] : prev.filter((x) => x !== s.value))}
                  />
                  <span className="jur-facet-option-label">{s.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* CNIL — facettes génériques Type / Nature de la délibération (saisie
          libre : valeurs non normalisées de façon fiable dans la doc
          officielle, contrairement à LODA/JORF). */}
      {source === 'legifrance' && fond === 'CNIL' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('cnil')} type="button">
            <span className="jur-facet-label">
              Type / Nature (CNIL)
              {(cnilType.length + cnilNatureDelib.length) > 0 && (
                <span className="jur-facet-count">{cnilType.length + cnilNatureDelib.length}</span>
              )}
            </span>
            <span className={`jur-facet-chevron${openFacets.cnil ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.cnil && (
            <div className="jur-facet-body">
              <span className="jur-label">Type de délibération</span>
              <div className="jur-tag-input-row">
                <input
                  className="jur-input--sm"
                  type="text"
                  placeholder="ex : AU"
                  value={cnilTypeInput}
                  onChange={(e) => setCnilTypeInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && cnilTypeInput.trim()) {
                      e.preventDefault();
                      setCnilType((prev) => prev.includes(cnilTypeInput.trim()) ? prev : [...prev, cnilTypeInput.trim()]);
                      setCnilTypeInput('');
                    }
                  }}
                />
                <button
                  type="button"
                  className="jur-btn jur-btn--sm"
                  onClick={() => { if (cnilTypeInput.trim()) { setCnilType((prev) => prev.includes(cnilTypeInput.trim()) ? prev : [...prev, cnilTypeInput.trim()]); setCnilTypeInput(''); } }}
                >
                  Ajouter
                </button>
              </div>
              {cnilType.length > 0 && (
                <div className="jur-tag-chips">
                  {cnilType.map((t) => (
                    <span key={t} className="jur-tag-chip">
                      {t}
                      <button type="button" aria-label="Retirer" onClick={() => setCnilType((prev) => prev.filter((x) => x !== t))}>×</button>
                    </span>
                  ))}
                </div>
              )}
              <span className="jur-label jur-label--spaced">Nature de la délibération</span>
              <div className="jur-tag-input-row">
                <input
                  className="jur-input--sm"
                  type="text"
                  placeholder="ex : SANCTION"
                  value={cnilNatureDelibInput}
                  onChange={(e) => setCnilNatureDelibInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && cnilNatureDelibInput.trim()) {
                      e.preventDefault();
                      setCnilNatureDelib((prev) => prev.includes(cnilNatureDelibInput.trim()) ? prev : [...prev, cnilNatureDelibInput.trim()]);
                      setCnilNatureDelibInput('');
                    }
                  }}
                />
                <button
                  type="button"
                  className="jur-btn jur-btn--sm"
                  onClick={() => { if (cnilNatureDelibInput.trim()) { setCnilNatureDelib((prev) => prev.includes(cnilNatureDelibInput.trim()) ? prev : [...prev, cnilNatureDelibInput.trim()]); setCnilNatureDelibInput(''); } }}
                >
                  Ajouter
                </button>
              </div>
              {cnilNatureDelib.length > 0 && (
                <div className="jur-tag-chips">
                  {cnilNatureDelib.map((t) => (
                    <span key={t} className="jur-tag-chip">
                      {t}
                      <button type="button" aria-label="Retirer" onClick={() => setCnilNatureDelib((prev) => prev.filter((x) => x !== t))}>×</button>
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Filtres structurels des fonds sans recherche plein texte
          (BOCC / débats / questions écrites / documents administratifs) */}
      {source === 'legifrance' && (fond === 'BOCC' || FONDS_SANS_RECHERCHE_TEXTUELLE.includes(fond)) && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('specialFilters')} type="button">
            <span className="jur-facet-label">
              {fond === 'BOCC' ? 'Filtres BOCC' : fond === 'DEBAT' ? 'Filtres débats' : fond === 'QUESTION_ECRITE' ? 'Filtres questions' : 'Filtres documents'}
            </span>
            <span className={`jur-facet-chevron${openFacets.specialFilters ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.specialFilters && (
            <div className="jur-facet-body">
              {fond === 'BOCC' && (
                <>
                  <span className="jur-label">IDCC (convention collective)</span>
                  <input
                    className="jur-input--sm"
                    type="text"
                    placeholder="ex : 1090"
                    value={boccIdcc}
                    onChange={(e) => setBoccIdcc(e.target.value)}
                  />
                  <p className="jur-facet-hint">Ce fonds n'a pas de recherche plein texte côté API : utilisez le titre (champ de recherche principal), l'IDCC et/ou la période.</p>
                </>
              )}
              {fond === 'DEBAT' && (
                <>
                  <span className="jur-label">Type d'assemblée</span>
                  {TYPES_PARLEMENT.map((t) => (
                    <label key={t.value} className={`jur-facet-option${typesPublication.includes(t.value) ? ' jur-facet-option--active' : ''}`}>
                      <input
                        type="checkbox"
                        checked={typesPublication.includes(t.value)}
                        onChange={(e) => setTypesPublication((prev) => e.target.checked ? [...prev, t.value] : prev.filter((x) => x !== t.value))}
                      />
                      <span className="jur-facet-option-label">{t.label}</span>
                    </label>
                  ))}
                  <p className="jur-facet-hint">Ce fonds n'a pas de recherche plein texte côté API : utilisez le type d'assemblée et/ou la période de publication.</p>
                </>
              )}
              {fond === 'QUESTION_ECRITE' && (
                <>
                  <span className="jur-label">Origine parlementaire</span>
                  {TYPES_PARLEMENT.map((t) => (
                    <label key={t.value} className={`jur-facet-option${parlementTypes.includes(t.value) ? ' jur-facet-option--active' : ''}`}>
                      <input
                        type="checkbox"
                        checked={parlementTypes.includes(t.value)}
                        onChange={(e) => setParlementTypes((prev) => e.target.checked ? [...prev, t.value] : prev.filter((x) => x !== t.value))}
                      />
                      <span className="jur-facet-option-label">{t.label}</span>
                    </label>
                  ))}
                  <p className="jur-facet-hint">Ce fonds n'a pas de recherche plein texte côté API : utilisez l'origine parlementaire et/ou la période de publication.</p>
                </>
              )}
              {fond === 'DOC_ADMIN' && (
                <>
                  <span className="jur-label">Année(s), séparées par virgule</span>
                  <input
                    className="jur-input--sm"
                    type="text"
                    placeholder="ex : 2023, 2024"
                    value={docAdminYears}
                    onChange={(e) => setDocAdminYears(e.target.value)}
                  />
                  <p className="jur-facet-hint">Ce fonds n'a pas de recherche plein texte côté API : filtrez par année de publication.</p>
                </>
              )}
            </div>
          )}
        </div>
      )}


      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('jurisdiction')} type="button">
            <span className="jur-facet-label">
              Juridiction
              {jurisdiction.length > 0 && <span className="jur-facet-count">{jurisdiction.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.jurisdiction ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.jurisdiction && (
            <div className="jur-facet-body">
              {jurisdictionOptions.map((j) => (
                <label
                  key={j.value}
                  className={`jur-facet-option${jurisdiction.includes(j.value) ? ' jur-facet-option--active' : ''}`}
                >
                  <input
                    type="checkbox"
                    checked={jurisdiction.includes(j.value)}
                    onChange={(e) => {
                      setJurisdiction((prev) =>
                        e.target.checked ? [...prev, j.value] : prev.filter((x) => x !== j.value)
                      );
                    }}
                  />
                  <span className="jur-facet-option-label">{j.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Publication (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('publication')} type="button">
            <span className="jur-facet-label">
              Publication
              {publication.length > 0 && <span className="jur-facet-count">{publication.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.publication ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.publication && (
            <div className="jur-facet-body">
              {publicationOptions.map((p) => (
                <label
                  key={p.value}
                  className={`jur-facet-option${publication.includes(p.value) ? ' jur-facet-option--active' : ''}`}
                >
                  <input
                    type="checkbox"
                    checked={publication.includes(p.value)}
                    onChange={(e) =>
                      setPublication((prev) =>
                        e.target.checked ? [...prev, p.value] : prev.filter((x) => x !== p.value)
                      )
                    }
                  />
                  <span className="jur-facet-option-label">{p.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Solution (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('solution')} type="button">
            <span className="jur-facet-label">
              Solution
              {solution.length > 0 && <span className="jur-facet-count">{solution.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.solution ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.solution && (
            <div className="jur-facet-body">
              {solutionOptions.map((s) => (
                <label
                  key={s.value}
                  className={`jur-facet-option${solution.includes(s.value) ? ' jur-facet-option--active' : ''}`}
                >
                  <input
                    type="checkbox"
                    checked={solution.includes(s.value)}
                    onChange={(e) =>
                      setSolution((prev) =>
                        e.target.checked ? [...prev, s.value] : prev.filter((x) => x !== s.value)
                      )
                    }
                  />
                  <span className="jur-facet-option-label">{s.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Type de décision (JUDILIBRE) — réutilise l'état `nature`, que le
          backend route vers le paramètre `type` côté JUDILIBRE. */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('nature')} type="button">
            <span className="jur-facet-label">
              Type de décision
              {nature.length > 0 && <span className="jur-facet-count">{nature.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.nature ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.nature && (
            <div className="jur-facet-body">
              {(decisionTypeOptions.length > 0 ? decisionTypeOptions : DECISION_TYPES_FALLBACK).map((t) => (
                <label key={t.value} className={`jur-facet-option${nature.includes(t.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={nature.includes(t.value)}
                    onChange={(e) => setNature((prev) => e.target.checked ? [...prev, t.value] : prev.filter((x) => x !== t.value))}
                  />
                  <span className="jur-facet-option-label">{t.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Chambre (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('chamber')} type="button">
            <span className="jur-facet-label">
              Chambre
              {chamber.length > 0 && <span className="jur-facet-count">{chamber.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.chamber ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.chamber && (
            <div className="jur-facet-body">
              {chamberOptions.length === 0 && !taxonomiesLoading[`chamber:${taxoContext}`] && (
                <p className="jur-facet-hint">Aucune donnée disponible pour ce contexte.</p>
              )}
              {taxonomiesLoading[`chamber:${taxoContext}`] && <p className="jur-facet-hint">Chargement…</p>}
              {chamberOptions.map((c) => (
                <label key={c.value} className={`jur-facet-option${chamber.includes(c.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={chamber.includes(c.value)}
                    onChange={(e) => setChamber((prev) => e.target.checked ? [...prev, c.value] : prev.filter((x) => x !== c.value))}
                  />
                  <span className="jur-facet-option-label">{c.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Formation (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('formation')} type="button">
            <span className="jur-facet-label">
              Formation
              {formation.length > 0 && <span className="jur-facet-count">{formation.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.formation ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.formation && (
            <div className="jur-facet-body">
              {formationOptions.length === 0 && !taxonomiesLoading[`formation:${taxoContext}`] && (
                <p className="jur-facet-hint">Aucune donnée disponible pour ce contexte.</p>
              )}
              {taxonomiesLoading[`formation:${taxoContext}`] && <p className="jur-facet-hint">Chargement…</p>}
              {formationOptions.map((f) => (
                <label key={f.value} className={`jur-facet-option${formation.includes(f.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={formation.includes(f.value)}
                    onChange={(e) => setFormation((prev) => e.target.checked ? [...prev, f.value] : prev.filter((x) => x !== f.value))}
                  />
                  <span className="jur-facet-option-label">{f.label}</span>
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Matière (JUDILIBRE) — saisie libre avec suggestions issues de la taxonomie */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('theme')} type="button">
            <span className="jur-facet-label">
              Matière
              {theme.length > 0 && <span className="jur-facet-count">{theme.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.theme ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.theme && (
            <div className="jur-facet-body">
              <div className="jur-tag-input-row">
                <input
                  className="jur-input--sm"
                  type="text"
                  list="jur-theme-datalist"
                  placeholder="ex : rupture conventionnelle"
                  value={themeInput}
                  onChange={(e) => setThemeInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && themeInput.trim()) {
                      e.preventDefault();
                      setTheme((prev) => prev.includes(themeInput.trim()) ? prev : [...prev, themeInput.trim()]);
                      setThemeInput('');
                    }
                  }}
                />
                <button
                  type="button"
                  className="jur-btn jur-btn--sm"
                  onClick={() => { if (themeInput.trim()) { setTheme((prev) => prev.includes(themeInput.trim()) ? prev : [...prev, themeInput.trim()]); setThemeInput(''); } }}
                >
                  Ajouter
                </button>
              </div>
              <datalist id="jur-theme-datalist">
                {themeOptions.map((t) => <option key={t.value} value={t.label} />)}
              </datalist>
              {theme.length > 0 && (
                <div className="jur-tag-chips">
                  {theme.map((t) => (
                    <span key={t} className="jur-tag-chip">
                      {t}
                      <button type="button" aria-label="Retirer" onClick={() => setTheme((prev) => prev.filter((x) => x !== t))}>×</button>
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Siège de juridiction (JUDILIBRE — pertinent pour CA / TJ) */}
      {source === 'judilibre' && (jurisdiction.length === 0 || jurisdiction.includes('ca') || jurisdiction.includes('tj')) && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('location')} type="button">
            <span className="jur-facet-label">
              Siège de juridiction
              {location.length > 0 && <span className="jur-facet-count">{location.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.location ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.location && (
            <div className="jur-facet-body">
              <div className="jur-tag-input-row">
                <input
                  className="jur-input--sm"
                  type="text"
                  list="jur-location-datalist"
                  placeholder="ex : Cour d'appel de Paris"
                  value={locationInput}
                  onChange={(e) => setLocationInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && locationInput.trim()) {
                      e.preventDefault();
                      setLocation((prev) => prev.includes(locationInput.trim()) ? prev : [...prev, locationInput.trim()]);
                      setLocationInput('');
                    }
                  }}
                />
                <button
                  type="button"
                  className="jur-btn jur-btn--sm"
                  onClick={() => { if (locationInput.trim()) { setLocation((prev) => prev.includes(locationInput.trim()) ? prev : [...prev, locationInput.trim()]); setLocationInput(''); } }}
                >
                  Ajouter
                </button>
              </div>
              <datalist id="jur-location-datalist">
                {locationOptions.map((l) => <option key={l.value} value={l.label} />)}
              </datalist>
              {location.length > 0 && (
                <div className="jur-tag-chips">
                  {location.map((l) => (
                    <span key={l} className="jur-tag-chip">
                      {l}
                      <button type="button" aria-label="Retirer" onClick={() => setLocation((prev) => prev.filter((x) => x !== l))}>×</button>
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Champ de recherche & opérateur (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('judField')} type="button">
            <span className="jur-facet-label">
              Rechercher dans
              {(judField.length > 0 || judOperator !== 'or') && <span className="jur-facet-count">{judField.length || '●'}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.judField ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.judField && (
            <div className="jur-facet-body">
              <span className="jur-label">Zones ciblées (défaut : tout le texte)</span>
              {FIELDS_JUDILIBRE.map((f) => (
                <label key={f.value} className={`jur-facet-option${judField.includes(f.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={judField.includes(f.value)}
                    onChange={(e) => setJudField((prev) => e.target.checked ? [...prev, f.value] : prev.filter((x) => x !== f.value))}
                  />
                  <span className="jur-facet-option-label">{f.label}</span>
                </label>
              ))}
              <span className="jur-label jur-label--spaced">Opérateur entre les termes</span>
              <select className="jur-facet-sort" value={judOperator} onChange={(e) => setJudOperator(e.target.value)}>
                {OPERATEURS_JUDILIBRE.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
              </select>
            </div>
          )}
        </div>
      )}

      {/* Documents associés (JUDILIBRE) */}
      {source === 'judilibre' && (
        <div className="jur-facet-group">
          <button className="jur-facet-summary" onClick={() => toggleFacet('fileType')} type="button">
            <span className="jur-facet-label">
              Documents associés
              {withFileOfType.length > 0 && <span className="jur-facet-count">{withFileOfType.length}</span>}
            </span>
            <span className={`jur-facet-chevron${openFacets.fileType ? ' jur-facet-chevron--open' : ''}`}>▸</span>
          </button>
          {openFacets.fileType && (
            <div className="jur-facet-body">
              {FILE_TYPES_JUDILIBRE.map((f) => (
                <label key={f.value} className={`jur-facet-option${withFileOfType.includes(f.value) ? ' jur-facet-option--active' : ''}`}>
                  <input
                    type="checkbox"
                    checked={withFileOfType.includes(f.value)}
                    onChange={(e) => setWithFileOfType((prev) => e.target.checked ? [...prev, f.value] : prev.filter((x) => x !== f.value))}
                  />
                  <span className="jur-facet-option-label">{f.label}</span>
                </label>
              ))}
              <label className="jur-facet-option jur-facet-option--toggle">
                <input
                  type="checkbox"
                  checked={particularInterest}
                  onChange={(e) => setParticularInterest(e.target.checked)}
                />
                <span className="jur-facet-option-label">Décisions à intérêt particulier uniquement</span>
              </label>
            </div>
          )}
        </div>
      )}

      {/* Période */}
      <div className="jur-facet-group">
        <button className="jur-facet-summary" onClick={() => toggleFacet('dates')} type="button">
          <span className="jur-facet-label">
            Période
            {(dateStart || dateEnd) && <span className="jur-facet-count">●</span>}
          </span>
          <span className={`jur-facet-chevron${openFacets.dates ? ' jur-facet-chevron--open' : ''}`}>▸</span>
        </button>
        {openFacets.dates && (
          <div className="jur-facet-body">
            {source === 'legifrance' && fond === 'ALL' ? (
              // L'API Légifrance renvoie une erreur serveur (500) si un
              // filtre de date est envoyé avec fond=ALL : aucune facette de
              // date n'existe pour « Tous les fonds ». On désactive donc les
              // champs plutôt que de laisser l'utilisateur croire, à tort,
              // que la période est appliquée.
              <p className="jur-facet-hint">Le filtre de période n'est pas disponible pour « Tous les fonds » (limitation de l'API Légifrance) : sélectionnez un fonds documentaire précis pour filtrer par date.</p>
            ) : (
              <div className="jur-facet-dates">
                <div className="jur-facet-date-row">
                  <span className="jur-label">Du</span>
                  <input
                    className="jur-input--date-sm"
                    type="date"
                    value={dateStart}
                    onChange={(e) => setDateStart(e.target.value)}
                  />
                </div>
                <div className="jur-facet-date-row">
                  <span className="jur-label">Au</span>
                  <input
                    className="jur-input--date-sm"
                    type="date"
                    value={dateEnd}
                    onChange={(e) => setDateEnd(e.target.value)}
                  />
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Tri */}
      <div className="jur-facet-group">
        <button className="jur-facet-summary" onClick={() => toggleFacet('tri')} type="button">
          <span className="jur-facet-label">Tri</span>
          <span className={`jur-facet-chevron${openFacets.tri ? ' jur-facet-chevron--open' : ''}`}>▸</span>
        </button>
        {openFacets.tri && (
          <div className="jur-facet-body">
            {source === 'legifrance' && !SORTS_BY_FOND[fond] ? (
              <p className="jur-facet-hint">Aucun tri n'est disponible pour ce fonds (limitation de l'API Légifrance) : les résultats sont classés par pertinence.</p>
            ) : (
              <select
                className="jur-facet-sort"
                value={sort}
                onChange={(e) => setSort(e.target.value)}
              >
                {(source === 'legifrance' ? (SORTS_BY_FOND[fond] ?? SORTS_LEGIFRANCE) : SORTS_JUDILIBRE).map((s) => (
                  <option key={s.value} value={s.value}>{s.label}</option>
                ))}
              </select>
            )}
          </div>
        )}
      </div>

      {activeFilterCount > 0 && (
        <button className="jur-btn jur-facets-reset" type="button" onClick={resetFilters}>
          Réinitialiser les filtres
        </button>
      )}

      <div className="jur-facets-footer">
        Sources officielles : Légifrance (DILA) &amp; JUDILIBRE (Cour de cassation).
      </div>
    </div>
  );

  return (
    <div className="jur-panel">
      {/* ── En-tête : identité de l'outil + fil d'Ariane global ───────────── */}
      <div className="jur-header">
        <div className="jur-header-left">
          <span className="jur-header-icon">⚖️</span>
          <span className="jur-header-title">Assistant juridique</span>
          <nav className="jur-breadcrumb" aria-label="Fil d'Ariane">
            <span className="jur-breadcrumb-sep">/</span>
            <span className="jur-breadcrumb-item">
              {source === 'legifrance' ? FOND_LABELS[fond] : 'JUDILIBRE'}
            </span>
            {hasSearched && query && (
              <>
                <span className="jur-breadcrumb-sep">/</span>
                <span className="jur-breadcrumb-item jur-breadcrumb-item--current" title={query}>
                  « {query} »
                </span>
              </>
            )}
          </nav>
        </div>
        <div className="jur-header-right">
          {globalControls && (
            <div className="jur-header-global-controls">
              {globalControls}
              {/* Gélule « Charger tous les articles » : n'apparaît que si le
                  document ouvert cite plus d'articles que le plafond par
                  défaut de la résolution inline (cf. DEFAULT_MAX_INLINE_CITATIONS
                  dans content/html-helpers.ts) — au-delà, seuls les N premiers
                  sont chargés automatiquement pour éviter une rafale de
                  requêtes réseau non désirée à l'ouverture d'un texte. */}
              {viewedDoc && !viewedDoc.allArticlesLoaded && !!viewedDoc.totalInlineArticles && (
                <button
                  className="artifacts-toggle-btn"
                  type="button"
                  onClick={loadAllArticles}
                  disabled={loadingAllArticles}
                  title={`Ce document cite ${viewedDoc.totalInlineArticles} articles : seuls les premiers sont chargés automatiquement. Cliquez pour charger le texte intégral de tous les articles cités.`}
                >
                  {loadingAllArticles ? (
                    <span className="jur-spinner jur-spinner--sm" />
                  ) : (
                    <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor"><path fillRule="evenodd" d="M2 3a1 1 0 011-1h10a1 1 0 110 2H3a1 1 0 01-1-1zm0 5a1 1 0 011-1h10a1 1 0 110 2H3a1 1 0 01-1-1zm1 4a1 1 0 100 2h6a1 1 0 100-2H3z" clipRule="evenodd"/></svg>
                  )}
                  {loadingAllArticles
                    ? 'Chargement…'
                    : `Charger tous les articles (${viewedDoc.totalInlineArticles})`}
                </button>
              )}
            </div>
          )}
          <button
            className={`jur-btn jur-btn--sm jur-filters-toggle${facetsOpen ? ' jur-filters-toggle--active' : ''}`}
            type="button"
            onClick={() => setFacetsOpen((v) => !v)}
            title="Afficher/masquer les filtres"
          >
            <span className="jur-burger-icon" aria-hidden="true"><span /><span /><span /></span>
            Filtres{activeFilterCount > 0 ? ` (${activeFilterCount})` : ''}
          </button>
          {/* ── Bouton Dossier enrichi ───────────────────────────────── */}
          <div className={`jur-dossier-btn-wrap${showDossier ? ' jur-dossier-btn-wrap--open' : ''}`}>
            <button
              className={`jur-dossier-main-btn${showDossier ? ' jur-dossier-main-btn--active' : ''}`}
              onClick={() => {
                setShowDossier((v) => !v);
                if (activeDossierId) setDossierView('detail');
                else setDossierView('list');
              }}
              title={activeDossier ? `${activeDossier.nom} · ${dossier.length} doc${dossier.length !== 1 ? 's' : ''}` : 'Gérer mes dossiers'}
            >
              <span className="jur-dossier-main-btn-icon">📂</span>
              <span className="jur-dossier-main-btn-text">
                <span className="jur-dossier-main-btn-label">
                  {activeDossier ? activeDossier.nom : 'Dossiers'}
                  {dossiers.length > 1 && (
                    <span className="jur-dossier-main-btn-count"> ({dossiers.length})</span>
                  )}
                </span>
                <span className="jur-dossier-main-btn-sub">
                  {activeDossier
                    ? `${dossier.length} doc${dossier.length !== 1 ? 's' : ''}`
                    : dossiers.length === 0 ? 'Aucun dossier' : `${dossiers.length} dossier${dossiers.length > 1 ? 's' : ''}`
                  }
                </span>
              </span>
            </button>
            {/* Mini-menu de changement de dossier */}
            {dossiers.length > 1 && (
              <div className="jur-dossier-switcher">
                {dossiers.map((dos) => (
                  <button
                    key={dos.id}
                    className={`jur-dossier-switcher-item${dos.id === activeDossierId ? ' jur-dossier-switcher-item--active' : ''}`}
                    onClick={() => {
                      setActiveDossierId(dos.id);
                      setShowDossier(true);
                      setDossierView('detail');
                    }}
                    title={`${dos.nom} · ${dos.items.length} doc${dos.items.length !== 1 ? 's' : ''}`}
                  >
                    <span className="jur-dossier-switcher-nom">{dos.nom}</span>
                    <span className="jur-dossier-switcher-count">{dos.items.length}</span>
                  </button>
                ))}
                <button
                  className="jur-dossier-switcher-item jur-dossier-switcher-item--new"
                  onClick={() => { createDossier(); setShowDossier(true); }}
                >+ Nouveau dossier</button>
              </div>
            )}
          </div>
          <button
            className={`jur-btn-icon jur-btn-icon--label${showFavorites ? ' jur-btn-icon--active' : ''}`}
            title={`Favoris (${favorites.length})`}
            onClick={() => setShowFavorites((v) => !v)}
          >
            <span>★</span>
            <span className="jur-btn-icon-label">
              Favoris{favorites.length > 0 && <span className="jur-badge jur-badge--chip">{favorites.length}</span>}
            </span>
          </button>
          <button className="jur-btn-icon" title="Masquer le panneau juridique (passer en vue Chat)" onClick={onClose}>✕</button>
        </div>
      </div>

      <div className={`jur-body${facetsOpen ? '' : ' jur-body--facets-closed'}`}>
        {/* ── Facettes : panneau repliable en overlay, masqué par défaut ──── */}
        {facetsOpen && (
          <div className="jur-facets-scrim" onClick={() => setFacetsOpen(false)} />
        )}
        {facetsPanel}

        {/* ── Résultats : pleine largeur au niveau recherche, colonne
           compacte (titres seuls) au niveau lecture une fois un document
           ouvert. ─────────────────────────────────────────────────────── */}
        <div className={`jur-results-col${viewedDoc ? ' jur-results-col--compact' : ''}`}>
          <form className="jur-searchbar" onSubmit={handleSubmit}>
            <div className="jur-searchbar-row">
              <input
                ref={queryInputRef}
                className="jur-input jur-input--query"
                type="text"
                placeholder={
                  source === 'legifrance'
                    ? FONDS_SANS_RECHERCHE_TEXTUELLE.includes(fond)
                      ? 'Fonds sans recherche plein texte — utilisez les filtres ci-contre'
                      : fond === 'BOCC'
                        ? 'Titre du texte (ex : convention collective)…'
                        : 'Ex : contrat de travail, licenciement…'
                    : 'Ex : licenciement faute grave…'
                }
                disabled={source === 'legifrance' && FONDS_SANS_RECHERCHE_TEXTUELLE.includes(fond)}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                list="jur-history-list"
                autoComplete="off"
              />
              <datalist id="jur-history-list">
                {history.map((h) => <option key={h} value={h} />)}
              </datalist>
              <button
                className="jur-btn jur-btn--primary"
                type="submit"
                disabled={loading || (fondRequiresQuery(source, fond) && !query.trim())}
              >
                {loading ? <span className="jur-spinner" /> : '🔍'}
              </button>
            </div>
            {!hasSearched && (
              <div className="jur-searchbar-hint">
                Recherchez un terme, un numéro d'article ou de pourvoi.
              </div>
            )}
          </form>

          {hasSearched && !loading && !error && (
            <div className="jur-results-summary">
              <span className="jur-results-summary-count">
                {total != null
                  ? <><strong>{total.toLocaleString('fr-FR')}</strong> résultat{total > 1 ? 's' : ''}</>
                  : <>{results.length} résultat{results.length > 1 ? 's' : ''}</>}
              </span>
            </div>
          )}

          <div className="jur-results">
            {error && (
              <div className="jur-error">
                <span className="jur-error-icon">⚠</span>
                <span>{error}</span>
              </div>
            )}

            {!error && hasSearched && !loading && results.length === 0 && (
              <div className="jur-empty">Aucun résultat pour cette recherche</div>
            )}

            {!error && !hasSearched && !loading && (
              <div className="jur-empty">
                Lancez une recherche pour afficher des résultats ici.
              </div>
            )}

            {loading && (
              <div className="jur-loading-list">
                {Array.from({ length: 5 }).map((_, i) => (
                  <div key={i} className="jur-result-skeleton">
                    <div className="jur-result-skeleton__kicker" />
                    <div className="jur-result-skeleton__title" />
                    <div className="jur-result-skeleton__meta" />
                    <div className="jur-result-skeleton__preview" />
                  </div>
                ))}
              </div>
            )}

            {!loading && results.map((r, i) => {
              const titre = extractTitle(r, resultsSource);
              const date = extractDate(r);
              const isCodeFond = ['CODE_DATE', 'CODE_ETAT'].includes(resultsFond);
              const isActive = viewedDoc?.id === extractResultId(r, isCodeFond);
              const resultId = resultsSource === 'judilibre' ? (r.id || '') : extractResultId(r, isCodeFond);
              const inDossier = isInDossier(resultId);
              const kicker = resultsSource === 'legifrance' ? FOND_LABELS[resultsFond] : (r.jurisdiction || 'JUDILIBRE');
              const titreParts = resultsSource === 'judilibre' ? titre.split('\n') : [titre];

              // Ligne juridiction / chambre / formation condensée (Judilibre)
              const courtLine = resultsSource === 'judilibre'
                ? [
                    r.jurisdiction,
                    r.chamber && r.chamber !== r.jurisdiction ? r.chamber : null,
                    r.formation && r.formation !== r.chamber ? r.formation : null,
                  ].filter(Boolean).join(' · ')
                : null;

              // Badge solution coloré sémantiquement (cassation/rejet/annulation/qpc…)
              const solMod = r.solution ? solutionModifier(r.solution) : null;

              // Thèmes : 3 affichés + compteur du reste
              // Judilibre → r.themes (taxonomie de matières native)
              // Légifrance → r.motsCles (mots-clés natifs, présents surtout
              // sur LODA/JORF/CONVCOLL ; absents pour la plupart des décisions
              // de jurisprudence JURI/CETAT/CONSTIT)
              const themes = resultsSource === 'judilibre' ? (r.themes || []) : (r.motsCles || []);
              const themesShown = themes.slice(0, 3).map(stripHtml);
              const themesHidden = themes.length - themesShown.length;

              // Indicateurs de publication (Bulletin / Rapport / Lettre / Communiqué)
              const pubs: string[] = Array.isArray(r.publication) ? r.publication : [];

              // Extrait surligné :
              // Judilibre  → highlights.text > highlights.all
              // Légifrance → sections[].extracts[].values (snippet <mark> du
              //              passage trouvé) > resumePrincipal (résumé global,
              //              sans surlignage, en dernier recours)
              const rawHighlight = resultsSource === 'judilibre'
                ? (r.highlights?.text?.[0] || r.highlights?.all?.[0] || null)
                : (extractLegifranceHighlight(r) || r.resumePrincipal?.[0] || null);

              // Score de pertinence normalisé (Judilibre)
              const score = typeof r.score === 'number' && r.score > 0 ? Math.min(r.score, 1) : null;

              return (
                <div
                  key={r.id || i}
                  className={`jur-result-item${isActive ? ' jur-result-item--active' : ''}`}
                  data-source={resultsSource}
                >
                  <button
                    className="jur-result-item-main"
                    onClick={() => openDocument(r, true, resultsFond, resultsSource)}
                    type="button"
                  >
                    <div className="jur-result-kicker">
                      <span className="jur-result-kicker-label">{kicker}</span>
                      {pubs.map((p) => (
                        <span key={p} className="jur-tag jur-tag--pub" title={PUB_LABELS[p] || p}>
                          {PUB_LABELS[p] || p}
                        </span>
                      ))}
                      {score !== null && (
                        <span
                          className="jur-result-score"
                          title={`Score de pertinence : ${Math.round(score * 100)} %`}
                        >
                          <span
                            className="jur-result-score-bar"
                            style={{ width: `${Math.round(score * 100)}%` }}
                          />
                        </span>
                      )}
                    </div>

                    <div className="jur-result-title">{titreParts[0]}</div>

                    {courtLine && (
                      <div className="jur-result-court">{courtLine}</div>
                    )}

                    {titreParts[1] && (
                      <div className="jur-result-subtitle">{titreParts[1]}</div>
                    )}

                    <div className="jur-result-meta">
                      {r.nature && <span className="jur-tag">{r.nature}</span>}
                      {r.etat && (
                        <span className={`jur-tag jur-tag--${r.etat === 'VIGUEUR' ? 'ok' : 'muted'}`}>
                          {r.etat}
                        </span>
                      )}
                      {r.solution && (
                        <span className={`jur-result-solution jur-result-solution--${solMod}`}>
                          {r.solution}
                        </span>
                      )}
                      {resultsSource === 'legifrance' && r.jurisdiction && (
                        <span className="jur-tag">{r.jurisdiction}</span>
                      )}
                      {date && <span className="jur-date">{date}</span>}
                      {/* A4 — dossier législatif (travaux préparatoires) lié
                          à ce texte, quand SearchResult.dossiersLegislatifs
                          en expose au moins un (cf. content/extract-dossier-
                          legislatif.ts pour les réserves sur ce champ). */}
                      {!!r.dossiersLegislatifs?.length && (
                        <button
                          type="button"
                          className="jur-tag jur-tag--dossier-legislatif"
                          title="Ouvrir les travaux préparatoires (exposé des motifs, débats…)"
                          onClick={(e) => {
                            e.stopPropagation();
                            const d = r.dossiersLegislatifs![0];
                            openDossierLegislatif(d.id, d.titre);
                          }}
                        >
                          📜 Travaux prépa.
                        </button>
                      )}
                    </div>

                    {themesShown.length > 0 && (
                      <div className="jur-result-themes">
                        {themesShown.map((theme) => (
                          <button
                            key={theme}
                            className="jur-result-theme-chip"
                            title={`Filtrer par thème : ${theme}`}
                            onClick={(e) => {
                              e.stopPropagation();
                              setQuery(theme);
                            }}
                          >
                            {theme}
                          </button>
                        ))}
                        {themesHidden > 0 && (
                          <span className="jur-result-theme-more">+{themesHidden}</span>
                        )}
                      </div>
                    )}

                    {rawHighlight && (
                      <div
                        className="jur-result-preview"
                        dangerouslySetInnerHTML={{ __html: sanitizeHtml(rawHighlight) }}
                      />
                    )}
                  </button>
                  {/* Bouton dossier sur la carte (visible au hover) */}
                  <button
                    className={`jur-result-dossier-btn${inDossier ? ' jur-result-dossier-btn--active' : ''}`}
                    title={inDossier ? 'Retirer du dossier' : 'Ajouter au dossier'}
                    onClick={async (e) => {
                      e.stopPropagation();
                      // Si le doc n'est pas encore chargé, on l'ouvre d'abord
                      if (!inDossier && viewedDoc?.id !== resultId) {
                        await openDocument(r, true, resultsFond, resultsSource);
                      } else if (inDossier) {
                        removeDossierItem(resultId);
                      } else if (viewedDoc) {
                        toggleDossier(viewedDoc);
                      }
                    }}
                  >
                    {inDossier ? '📂' : '📁'}
                  </button>
                </div>
              );
            })}

            {/* Pagination */}
            {!loading && results.length > 0 && (
              <div className="jur-pagination">
                {total != null && (
                  <span className="jur-pagination-total">
                    {total.toLocaleString('fr-FR')} résultat{total > 1 ? 's' : ''}
                  </span>
                )}
                <div className="jur-pagination-nav">
                  <button
                    className="jur-btn jur-btn--sm"
                    disabled={page <= 1}
                    onClick={() => doSearch(page - 1)}
                  >‹</button>
                  <span className="jur-pagination-page">
                    {page}{totalPages ? ` / ${totalPages}` : ''}
                  </span>
                  <button
                    className="jur-btn jur-btn--sm"
                    disabled={totalPages != null && page >= totalPages}
                    onClick={() => doSearch(page + 1)}
                  >›</button>
                </div>
              </div>
            )}
          </div>

          {/* Favoris */}
          {showFavorites && (
            <div className="jur-favorites">
              <div className="jur-favorites-header">
                ★ Favoris ({favorites.length})
                <button className="jur-btn-link" onClick={() => setShowFavorites(false)}>✕</button>
              </div>
              {favorites.length === 0 && (
                <div className="jur-empty">Aucun favori pour le moment</div>
              )}
              {favorites.map((fav) => {
                const favTitreParts = fav.titre.split('\n');
                return (
                  <button
                    key={fav.id}
                    className="jur-result-item"
                    type="button"
                    onClick={() => { setViewedDoc(fav); setShowFavorites(false); }}
                  >
                    <div className="jur-result-title">{favTitreParts[0]}</div>
                    {favTitreParts[1] && (
                      <div className="jur-result-subtitle">{favTitreParts[1]}</div>
                    )}
                    <div className="jur-result-meta">
                      <span className="jur-tag">{fav.source}</span>
                      <span className="jur-tag">{DOC_TYPE_LABELS[fav.type]}</span>
                    </div>
                  </button>
                );
              })}
            </div>
          )}

          {/* ── Dossier / Cartable ──────────────────────────────────────────── */}
          {showDossier && (
            <div className="jur-dossier-panel">

              {/* ── Vue liste des dossiers ─────────────────────────────── */}
              {dossierView === 'list' && (
                <>
                  <div className="jur-dossier-header">
                    <span className="jur-dossier-title">📂 Mes dossiers</span>
                    <div className="jur-dossier-header-actions">
                      <button className="jur-btn-link" onClick={createDossier} title="Nouveau dossier">+ Nouveau</button>
                      <button className="jur-btn-link" onClick={() => setShowDossier(false)}>✕</button>
                    </div>
                  </div>

                  {dossiers.length === 0 && (
                    <div className="jur-dossier-empty">
                      <div className="jur-dossier-empty-icon">📁</div>
                      <div>Aucun dossier. Cliquez sur <strong>+ Nouveau</strong> pour commencer.</div>
                    </div>
                  )}

                  <div className="jur-dossier-folder-list">
                    {dossiers.map((dos) => (
                      <div
                        key={dos.id}
                        className={`jur-dossier-folder-item${dos.id === activeDossierId ? ' jur-dossier-folder-item--active' : ''}`}
                        onClick={() => { setActiveDossierId(dos.id); setDossierView('detail'); }}
                      >
                        <span className="jur-dossier-folder-icon">📁</span>
                        <div className="jur-dossier-folder-info">
                          {renamingId === dos.id ? (
                            <input
                              className="jur-dossier-rename-input"
                              value={renameValue}
                              autoFocus
                              onClick={(e) => e.stopPropagation()}
                              onChange={(e) => setRenameValue(e.target.value)}
                              onBlur={() => renameDossier(dos.id, renameValue)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') renameDossier(dos.id, renameValue);
                                if (e.key === 'Escape') setRenamingId(null);
                                e.stopPropagation();
                              }}
                            />
                          ) : (
                            <span className="jur-dossier-folder-nom">{dos.nom}</span>
                          )}
                          <span className="jur-dossier-folder-count">{dos.items.length} doc{dos.items.length !== 1 ? 's' : ''}</span>
                        </div>
                        <div className="jur-dossier-folder-actions" onClick={(e) => e.stopPropagation()}>
                          <button
                            className="jur-btn-icon jur-btn-icon--xs"
                            title="Renommer"
                            onClick={(e) => { e.stopPropagation(); setRenamingId(dos.id); setRenameValue(dos.nom); }}
                          >✏️</button>
                          <button
                            className="jur-btn-icon jur-btn-icon--xs"
                            title="Supprimer ce dossier"
                            onClick={(e) => { e.stopPropagation(); if (window.confirm(`Supprimer « ${dos.nom} » ?`)) deleteDossier(dos.id); }}
                          >🗑</button>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              )}

              {/* ── Vue détail d'un dossier ────────────────────────────── */}
              {dossierView === 'detail' && activeDossier && (
                <>
                  <div className="jur-dossier-header">
                    <button className="jur-btn-icon jur-btn-icon--xs" title="Retour aux dossiers" onClick={() => { setDossierView('list'); setRenamingId(null); }}>←</button>
                    {renamingId === activeDossier.id ? (
                      <input
                        className="jur-dossier-rename-input jur-dossier-rename-input--header"
                        value={renameValue}
                        autoFocus
                        onChange={(e) => setRenameValue(e.target.value)}
                        onBlur={() => renameDossier(activeDossier.id, renameValue)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') renameDossier(activeDossier.id, renameValue);
                          if (e.key === 'Escape') setRenamingId(null);
                        }}
                      />
                    ) : (
                      <span
                        className="jur-dossier-title jur-dossier-title--editable"
                        title="Double-cliquer pour renommer"
                        onDoubleClick={() => { setRenamingId(activeDossier.id); setRenameValue(activeDossier.nom); }}
                      >
                        📁 {activeDossier.nom}
                        <button
                          className="jur-btn-icon jur-btn-icon--xs jur-dossier-rename-btn"
                          title="Renommer ce dossier"
                          onClick={() => { setRenamingId(activeDossier.id); setRenameValue(activeDossier.nom); }}
                        >✏️</button>
                      </span>
                    )}
                    <div className="jur-dossier-header-actions">
                      {dossier.length > 0 && (
                        <button className="jur-btn-link jur-btn-link--danger" onClick={clearDossier} title="Vider le dossier">Vider</button>
                      )}
                      {dossier.length > 0 && (
                        <button className="jur-btn-link" onClick={exportDossierMarkdown} title="Exporter en Markdown">⬇ Export</button>
                      )}
                      <button className="jur-btn-link" onClick={() => setShowDossier(false)}>✕</button>
                    </div>
                  </div>

                  {dossier.length === 0 && (
                    <div className="jur-dossier-empty">
                      <div className="jur-dossier-empty-icon">📄</div>
                      <div>Ouvrez un document puis cliquez sur <strong>📁 Dossier</strong> pour l'ajouter ici.</div>
                    </div>
                  )}

                  {dossier.length > 0 && (
                    <>
                      <div className="jur-dossier-list">
                        {dossier.map((item, i) => (
                          <div
                            key={item.id}
                            className={`jur-dossier-item jur-dossier-item--clickable${viewedDoc?.id === item.id ? ' jur-dossier-item--active' : ''}`}
                            onClick={() => openDossierItem(item)}
                            title="Ouvrir ce document"
                          >
                            <span className="jur-dossier-item-num">{i + 1}</span>
                            <div className="jur-dossier-item-info">
                              <div className="jur-dossier-item-titre">{item.titre}</div>
                              <div className="jur-dossier-item-meta">
                                <span className="jur-tag jur-tag--xs">{item.source}</span>
                                <span className="jur-tag jur-tag--xs">{DOC_TYPE_LABELS[item.type as ViewedDoc['type']]}</span>
                              </div>
                            </div>
                            <button
                              className="jur-dossier-item-remove"
                              title="Retirer du dossier"
                              onClick={(e) => { e.stopPropagation(); removeDossierItem(item.id); }}
                            >✕</button>
                          </div>
                        ))}
                      </div>

                      {/* Analyse LLM groupée */}
                      <div className="jur-dossier-llm">
                        <div className="jur-dossier-llm-title">
                          ⚖️ Analyser le dossier avec l'IA
                          <span className="jur-dossier-llm-count">{dossier.length} doc{dossier.length > 1 ? 's' : ''}</span>
                        </div>
                        <div className="jur-dossier-llm-modes">
                          <button
                            className={`jur-dossier-llm-btn${dossierLlmMode === 'synthese' ? ' jur-dossier-llm-btn--loading' : ''}`}
                            disabled={dossierLlmLoading}
                            onClick={() => sendDossierToLlm('synthese')}
                          >
                            {dossierLlmMode === 'synthese' ? <span className="jur-spinner jur-spinner--sm" /> : '📋'}
                            <div className="jur-dossier-llm-btn-label"><strong>Synthèse</strong><span>Résumé unifié</span></div>
                          </button>
                          <button
                            className={`jur-dossier-llm-btn${dossierLlmMode === 'contradictoire' ? ' jur-dossier-llm-btn--loading' : ''}`}
                            disabled={dossierLlmLoading}
                            onClick={() => sendDossierToLlm('contradictoire')}
                          >
                            {dossierLlmMode === 'contradictoire' ? <span className="jur-spinner jur-spinner--sm" /> : '⚔️'}
                            <div className="jur-dossier-llm-btn-label"><strong>Contradictoire</strong><span>Convergences / tensions</span></div>
                          </button>
                          <button
                            className={`jur-dossier-llm-btn${dossierLlmMode === 'points_cles' ? ' jur-dossier-llm-btn--loading' : ''}`}
                            disabled={dossierLlmLoading}
                            onClick={() => sendDossierToLlm('points_cles')}
                          >
                            {dossierLlmMode === 'points_cles' ? <span className="jur-spinner jur-spinner--sm" /> : '🎯'}
                            <div className="jur-dossier-llm-btn-label"><strong>Points clés</strong><span>Doc par doc + synthèse</span></div>
                          </button>
                        </div>
                        <div className="jur-dossier-llm-disclaimer">
                          L'analyse IA est un outil d'aide. Le professionnel reste seul responsable de son interprétation.
                          La réponse s'affiche dans le chat, à droite — un panneau « Artéfacts » s'y ouvre automatiquement.
                        </div>

                        {/* ── Historique des analyses de ce dossier ──────────
                            Chaque analyse est désormais rattachée au dossier
                            lui-même (pas seulement au chat, qui repart à zéro
                            à la prochaine demande) et conservée datée — pensé
                            pour accumuler plusieurs analyses dans le temps
                            plutôt que d'écraser la précédente. */}
                        {!!activeDossier?.analyses?.length && (
                          <div className="jur-dossier-history">
                            <button
                              className="jur-dossier-history-toggle"
                              onClick={() => setShowAnalysesHistory((v) => !v)}
                            >
                              <span>{showAnalysesHistory ? '▾' : '▸'} Historique des analyses</span>
                              <span className="jur-dossier-history-count">{activeDossier.analyses.length}</span>
                            </button>
                            {showAnalysesHistory && (
                              <ul className="jur-dossier-history-list">
                                {[...activeDossier.analyses].reverse().map((a) => (
                                  <li key={a.id} className="jur-dossier-history-item">
                                    <div className="jur-dossier-history-item-main">
                                      <span className="jur-dossier-history-item-icon">{DOSSIER_LLM_MODE_LABELS[a.mode].icon}</span>
                                      <div className="jur-dossier-history-item-info">
                                        <div className="jur-dossier-history-item-title">{DOSSIER_LLM_MODE_LABELS[a.mode].label}</div>
                                        <div className="jur-dossier-history-item-date">
                                          {new Date(a.createdAt).toLocaleString('fr-FR', { day: '2-digit', month: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit' })}
                                          {' · '}{a.itemTitres.length} doc{a.itemTitres.length > 1 ? 's' : ''}
                                        </div>
                                      </div>
                                    </div>
                                    <div className="jur-dossier-history-item-actions">
                                      <button
                                        className="jur-btn-link"
                                        title="Revoir cette analyse dans le chat"
                                        onClick={() => reopenDossierAnalysis(a)}
                                      >Revoir</button>
                                      <button
                                        className="jur-btn-link jur-btn-link--danger"
                                        title="Supprimer cette analyse"
                                        onClick={() => removeDossierAnalysis(a.id)}
                                      >✕</button>
                                    </div>
                                  </li>
                                ))}
                              </ul>
                            )}
                          </div>
                        )}
                      </div>
                    </>
                  )}
                </>
              )}
            </div>
          )}
        </div>

        {/* ── Colonne 3 : visionneuse (Doctrine) ─────────────────────────── */}
        <div className={`jur-viewer${!viewedDoc && !loadingDoc && !docError ? ' jur-viewer--hidden' : ''}`}>
          {/* État vide */}
          {!viewedDoc && !loadingDoc && !docError && (
            <div className="jur-viewer-empty">
              <div className="jur-viewer-empty-icon">📄</div>
              <div className="jur-viewer-empty-text">
                Sélectionnez un résultat pour afficher le texte intégral, avec son contexte et ses références.
              </div>
            </div>
          )}

          {/* Chargement doc */}
          {loadingDoc && (
            <div className="jur-viewer-loading">
              <div className="jur-spinner jur-spinner--lg" />
              <span>Chargement du document…</span>
            </div>
          )}

          {/* Erreur doc */}
          {docError && (
            <div className="jur-error jur-error--doc">
              <span className="jur-error-icon">⚠</span>
              {docError}
            </div>
          )}

          {/* Document affiché */}
          {viewedDoc && !loadingDoc && (
            <>
              {/* En-tête du document : fil d'Ariane + actions */}
              <div className="jur-viewer-header">
                {/* Ligne 1 : fil d'Ariane */}
                <div className="jur-viewer-header-row">
                  <button
                    className="jur-btn-results-back"
                    title="Retour aux résultats (Échap)"
                    onClick={closeDocument}
                    type="button"
                  >
                    ← Retour aux résultats
                  </button>
                  {docStack.length > 0 && (
                    <button
                      className="jur-btn-back"
                      title={`Retour : ${docStack[docStack.length - 1].titre.split('\n')[0]}`}
                      onClick={goBack}
                    >
                      ←
                    </button>
                  )}
                  <nav className="jur-viewer-breadcrumb" aria-label="Document affiché">
                    <span className="jur-viewer-breadcrumb-item jur-viewer-breadcrumb-item--current" title={viewedDoc.titre.split('\n')[0]}>
                      {viewedDoc.titre.split('\n')[0]}
                    </span>
                  </nav>
                </div>
                {/* Ligne 2 : tags source/type + id + boutons action */}
                <div className="jur-viewer-meta">
                  <span className="jur-tag jur-tag--source">{viewedDoc.source}</span>
                  <span className="jur-tag">{DOC_TYPE_LABELS[viewedDoc.type]}</span>
                  <span className="jur-viewer-id" title={viewedDoc.source === 'judilibre' ? judilibreDisplayRef(viewedDoc.raw, viewedDoc.id) : viewedDoc.id}>
                    {viewedDoc.source === 'judilibre'
                      ? judilibreDisplayRef(viewedDoc.raw, viewedDoc.id)
                      : `${viewedDoc.id.slice(0, 24)}${viewedDoc.id.length > 24 ? '…' : ''}`}
                  </span>
                  <div className="jur-viewer-actions-top">
                    {/* A4 — dossier législatif (travaux préparatoires) associé
                        à ce texte, reporté depuis SearchResult.dossiersLegislatifs
                        à l'ouverture (cf. useDocumentViewer.ts::openDocument). */}
                    {!!viewedDoc.dossiersLegislatifs?.length && (
                      <button
                        className="jur-btn-icon jur-btn-icon--label"
                        title="Ouvrir les travaux préparatoires (exposé des motifs, débats…)"
                        onClick={() => {
                          const d = viewedDoc.dossiersLegislatifs![0];
                          openDossierLegislatif(d.id, d.titre);
                        }}
                      >
                        <span>📜</span>
                        <span className="jur-btn-icon-label">Travaux prépa.</span>
                      </button>
                    )}
                    {/* Ajouter au dossier */}
                    <button
                      className={`jur-btn-icon jur-btn-icon--label${isInDossier(viewedDoc.id) ? ' jur-btn-icon--active' : ''}`}
                      title={isInDossier(viewedDoc.id) ? 'Retirer du dossier' : 'Ajouter au dossier'}
                      onClick={() => toggleDossier(viewedDoc)}
                    >
                      <span>{isInDossier(viewedDoc.id) ? '📂' : '📁'}</span>
                      <span className="jur-btn-icon-label">Dossier</span>
                    </button>
                    {/* Timeline */}
                    {(viewedDoc.source === 'legifrance') && (
                      <button
                        className={`jur-btn-icon jur-btn-icon--label${showTimeline ? ' jur-btn-icon--active' : ''}`}
                        title="Chronologie des versions"
                        onClick={showTimeline ? () => setShowTimeline(false) : loadTimeline}
                      >
                        <span>📅</span>
                        <span className="jur-btn-icon-label">Versions</span>
                      </button>
                    )}
                    {/* Annotations */}
                    <button
                      className={`jur-btn-icon jur-btn-icon--label${showAnnotations ? ' jur-btn-icon--active' : ''}`}
                      title={`Annotations (${currentDocAnnotations.length})`}
                      onClick={() => setShowAnnotations((v) => !v)}
                    >
                      <span>✏️</span>
                      <span className="jur-btn-icon-label">
                        Notes{currentDocAnnotations.length > 0 ? ` (${currentDocAnnotations.length})` : ''}
                      </span>
                    </button>
                    {/* Favori */}
                    <button
                      className={`jur-btn-icon${isFavorite ? ' jur-btn-icon--active' : ''}`}
                      title={isFavorite ? 'Retirer des favoris' : 'Ajouter aux favoris'}
                      onClick={() => toggleFavorite(viewedDoc)}
                    >★</button>
                    <button
                      className="jur-btn-icon"
                      title="Exporter"
                      onClick={exportMarkdown}
                    >⬇</button>
                  </div>
                </div>
              </div>

              {/* ── Timeline ─────────────────────────────────────────────── */}
              {showTimeline && (
                <div className="jur-timeline">
                  {/* En-tête avec onglets */}
                  <div className="jur-timeline-header">
                    <div className="jur-timeline-tabs">
                      <button
                        className={`jur-timeline-tab${timelineTab === 'doc' ? ' jur-timeline-tab--active' : ''}`}
                        onClick={() => setTimelineTab('doc')}
                      >
                        📅 Ce document
                      </button>
                      <button
                        className={`jur-timeline-tab${timelineTab === 'dossier' ? ' jur-timeline-tab--active' : ''}`}
                        onClick={() => setTimelineTab('dossier')}
                        disabled={!activeDossier}
                        title={activeDossier ? `Dossier : ${activeDossier.nom}` : 'Aucun dossier actif'}
                      >
                        📂 {activeDossier ? activeDossier.nom : 'Dossier'}
                        {activeDossier && <span className="jur-timeline-tab-count">{activeDossier.items.length}</span>}
                      </button>
                    </div>
                    <button className="jur-btn-link" onClick={() => setShowTimeline(false)}>✕</button>
                  </div>

                  {/* ── Onglet "Ce document" : timeline horizontale ── */}
                  {timelineTab === 'doc' && (
                    <>
                      {timelineLoading && <div className="jur-timeline-loading"><div className="jur-spinner" /></div>}
                      {!timelineLoading && timelineData.length === 0 && (
                        <div className="jur-timeline-empty">Aucune version disponible pour ce document.</div>
                      )}
                      {!timelineLoading && timelineData.length > 0 && (
                        <div className="jur-timeline-h-scroll">
                          <div className="jur-timeline-h-track">
                            <div className="jur-timeline-h-line" />
                            {timelineData.map((v, i) => {
                              const dDebut = parseChronoDate(v.dateDebut);
                              const fmtDate = (d: Date | null) =>
                                d ? d.toLocaleDateString('fr-FR', { day: '2-digit', month: 'short', year: 'numeric' }) : '?';
                              const fmtYear = (d: Date | null) =>
                                d ? d.getFullYear().toString() : '?';
                              const isVigueur = v.isEndVersion || v.etat === 'VIGUEUR';
                              const tooltipLines = [
                                fmtDate(dDebut),
                                isVigueur ? 'En vigueur' : 'Modifié',
                                ...(v.modificateurs || []).slice(0, 3).map((m) =>
                                  `${m.action ? m.action + ' — ' : ''}${m.title || m.id}`
                                ),
                              ].join('\n');
                              return (
                                <div
                                  key={v.id || i}
                                  className={`jur-timeline-h-node${isVigueur ? ' jur-timeline-h-node--vigueur' : ''}`}
                                  title={tooltipLines}
                                >
                                  <div className="jur-timeline-h-dot" />
                                  <div className="jur-timeline-h-label">
                                    <span className="jur-timeline-h-year">{fmtYear(dDebut)}</span>
                                    {isVigueur && <span className="jur-timeline-h-badge">✓</span>}
                                  </div>
                                </div>
                              );
                            })}
                          </div>
                        </div>
                      )}
                    </>
                  )}

                  {/* ── Onglet "Dossier actif" : liste chronologique ── */}
                  {timelineTab === 'dossier' && activeDossier && (
                    <div className="jur-timeline-dossier">
                      {activeDossier.items.length === 0 && (
                        <div className="jur-timeline-empty">Le dossier « {activeDossier.nom} » est vide.</div>
                      )}
                      {[...activeDossier.items]
                        .sort((a, b) => a.addedAt - b.addedAt)
                        .map((item, i) => (
                          <div key={item.id} className="jur-timeline-dossier-item">
                            <div className="jur-timeline-dossier-num">{i + 1}</div>
                            <div className="jur-timeline-dossier-body">
                              <span className="jur-timeline-dossier-titre">{item.titre}</span>
                              <span className="jur-timeline-dossier-meta">
                                <span className={`jur-tag jur-tag--xs jur-tag--${item.source === 'legifrance' ? 'legi' : 'judi'}`}>
                                  {item.source === 'legifrance' ? 'Légifrance' : 'Judilibre'}
                                </span>
                                {new Date(item.addedAt).toLocaleDateString('fr-FR', { day: '2-digit', month: 'short', year: 'numeric' })}
                              </span>
                            </div>
                          </div>
                        ))}
                    </div>
                  )}
                  {timelineTab === 'dossier' && !activeDossier && (
                    <div className="jur-timeline-empty">Aucun dossier actif. Créez ou sélectionnez un dossier.</div>
                  )}
                </div>
              )}

              {/* ── Panneau Annotations ───────────────────────────────────── */}
              {showAnnotations && (
                <div className="jur-annotations-panel">
                  <div className="jur-annotations-header">
                    <span>✏️ Annotations ({currentDocAnnotations.length})</span>
                    <div className="jur-annotations-controls">
                      <label className="jur-ann-toggle" title="Activer/désactiver la détection de citations">
                        <input
                          type="checkbox"
                          checked={highlightCitations}
                          onChange={(e) => setHighlightCitations(e.target.checked)}
                        />
                        <span>Citations auto</span>
                      </label>
                      <button className="jur-btn-link" onClick={() => setShowAnnotations(false)}>✕</button>
                    </div>
                  </div>
                  <div className="jur-annotations-hint">
                    Sélectionnez du texte dans le document pour annoter.
                  </div>
                  {currentDocAnnotations.length === 0 && (
                    <div className="jur-annotations-empty">Aucune annotation pour ce document.</div>
                  )}
                  {currentDocAnnotations.map((ann, idx) => {
                    const isEditing = editingAnnotationId === ann.id;
                    return (
                    <div
                      key={ann.id}
                      className={`jur-annotation-item jur-annotation-item--${ann.color}${isEditing ? '' : ' jur-annotation-item--clickable'}`}
                      onClick={() => !isEditing && scrollToAnnotation(ann.id)}
                      title={isEditing ? undefined : "Aller au surlignage"}
                    >
                      <div className="jur-annotation-header">
                        <span className={`jur-ann-badge jur-ann-badge--${ann.color}`}>{idx + 1}</span>
                        <div className="jur-annotation-text">« {ann.text.slice(0, 120)}{ann.text.length > 120 ? '…' : ''} »</div>
                      </div>
                      {isEditing ? (
                        <div className="jur-annotation-edit" onClick={(e) => e.stopPropagation()}>
                          <textarea
                            className="jur-ann-edit-input"
                            value={editingAnnotationNote}
                            onChange={(e) => setEditingAnnotationNote(e.target.value)}
                            placeholder="Note (optionnel)…"
                            autoFocus
                            rows={3}
                          />
                          <div className="jur-ann-edit-btns">
                            <button className="jur-btn-link jur-btn-link--primary" onClick={() => saveEditAnnotation(ann.id)}>✓ Valider</button>
                            <button className="jur-btn-link" onClick={cancelEditAnnotation}>✕ Annuler</button>
                          </div>
                        </div>
                      ) : (
                        ann.note && <div className="jur-annotation-note">{ann.note}</div>
                      )}
                      <div className="jur-annotation-meta">
                        <span>{new Date(ann.createdAt).toLocaleDateString('fr-FR')}</span>
                        <div className="jur-ann-recolor" onClick={(e) => e.stopPropagation()}>
                          {(['yellow', 'green', 'blue', 'pink'] as AnnotationColor[]).map((c) => (
                            <button
                              key={c}
                              className={`jur-ann-color jur-ann-color--${c}${ann.color === c ? ' jur-ann-color--selected' : ''}`}
                              title={`Couleur : ${c}`}
                              onClick={() => changeAnnotationColor(ann.id, c)}
                            />
                          ))}
                        </div>
                        {!isEditing && (
                          <button
                            className="jur-btn-link"
                            title="Modifier la note"
                            onClick={(e) => { e.stopPropagation(); startEditAnnotation(ann); }}
                          >✏️ Modifier</button>
                        )}
                        <button className="jur-btn-link jur-btn-link--danger" onClick={(e) => { e.stopPropagation(); deleteAnnotation(ann.id); }}>Supprimer</button>
                      </div>
                    </div>
                    );
                  })}
                </div>
              )}

              {/* Corps du document + table des matières flottante */}
              <div className="jur-viewer-body-row">
                <div
                  className="jur-viewer-content jur-viewer-content--html"
                  ref={viewerContentRef}
                  onMouseUp={handleTextSelection}
                  onClick={(e) => {
                    // Lien de secours (rapprochement/texte appliqué sans
                    // id/url résolu par l'API — cf. extract-judilibre.ts) :
                    // lance une recherche interne plutôt que d'ouvrir un
                    // document précis. Vérifié EN PREMIER : ces liens ne
                    // portent pas `data-cite-id`, mais autant être explicite
                    // pour ne pas dépendre de l'ordre des conditions dans
                    // handleCitationClick.
                    const target = e.target as HTMLElement;
                    const searchEl = target.closest('[data-search-query]') as HTMLElement | null;
                    if (searchEl) {
                      e.preventDefault();
                      const q = searchEl.dataset.searchQuery;
                      const src = searchEl.dataset.searchSource as Source | undefined;
                      const fnd = searchEl.dataset.searchFond as Fond | undefined;
                      if (q && src) searchFor(q, { source: src, fond: fnd });
                      return;
                    }
                    handleCitationClick(e);
                  }}
                  onMouseOver={handleLinkMouseEnter}
                  onMouseOut={handleLinkMouseLeave}
                >
                {/* Bulle de prévisualisation des liens */}
                {linkPreview && (
                  <div
                    className={`jur-link-preview${linkPreview.error ? ' jur-link-preview--error' : ''}`}
                    style={{
                      left: Math.min(linkPreview.x, window.innerWidth - 440),
                      top: linkPreview.y,
                    }}
                  >
                    {linkPreview.content === null
                      ? <span className="jur-link-preview-loading">Chargement…</span>
                      : <span className="jur-link-preview-text">{linkPreview.content}</span>
                    }
                  </div>
                )}
                {/* Popover d'annotation */}
                {pendingAnnotation && (
                  <div
                    className="jur-ann-popover"
                    ref={annPopoverRef}
                    style={{ left: Math.max(8, pendingAnnotation.x - 120), top: pendingAnnotation.y }}
                  >
                    <div className="jur-ann-popover-colors">
                      {(['yellow', 'green', 'blue', 'pink'] as AnnotationColor[]).map((c) => (
                        <button
                          key={c}
                          className={`jur-ann-color jur-ann-color--${c}${annotationColor === c ? ' jur-ann-color--selected' : ''}`}
                          onClick={(e) => { e.stopPropagation(); setAnnotationColor(c); }}
                          title={c}
                        />
                      ))}
                    </div>
                    <input
                      className="jur-ann-note-input"
                      placeholder="Note (optionnel)…"
                      value={annotationNote}
                      onChange={(e) => setAnnotationNote(e.target.value)}
                      onKeyDown={(e) => { if (e.key === 'Enter') saveAnnotation(); if (e.key === 'Escape') setPendingAnnotation(null); }}
                      autoFocus
                    />
                    <div className="jur-ann-popover-btns">
                      <button className="jur-btn jur-btn--sm jur-btn--primary" onClick={saveAnnotation}>Annoter</button>
                      <button className="jur-btn jur-btn--sm" onClick={() => setPendingAnnotation(null)}>Annuler</button>
                    </div>
                  </div>
                )}
                  <div
                    className="jur-doc-body"
                    dangerouslySetInnerHTML={{
                      __html: getAnnotatedHtml(viewedDoc.content.body, currentDocAnnotations),
                    }}
                  />
                </div>

                {/* ── Plan interactif du document (sommaire façon Dalloz) ── */}
                {tocTree.length > 0 && (
                  <nav
                    ref={tocNavRef}
                    className={`jur-toc${tocCollapsed ? ' jur-toc--collapsed' : ''}`}
                    aria-label="Plan du document"
                  >
                    <button
                      className="jur-toc-toggle"
                      title={tocCollapsed ? 'Afficher le plan' : 'Réduire le plan'}
                      onClick={() => setTocCollapsed((v) => !v)}
                    >
                      <span>📑</span>
                      {!tocCollapsed && (
                        <span className="jur-toc-toggle-label">
                          {viewedDoc.type === 'code' ? 'Plan du code' : 'Plan du document'}
                        </span>
                      )}
                      <span className="jur-toc-toggle-chevron">{tocCollapsed ? '‹' : '›'}</span>
                    </button>
                    {!tocCollapsed && (
                      <TocTreeList
                        nodes={tocTree}
                        activeId={activeTocId}
                        expandedIds={expandedTocIds}
                        onToggle={toggleTocNode}
                        onSelect={scrollToTocEntry}
                      />
                    )}
                  </nav>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
