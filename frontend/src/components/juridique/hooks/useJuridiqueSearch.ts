// ============================================================================
// Juridix — Espace "Recherche juridique" — Hook useJuridiqueSearch
// ============================================================================
// État et logique du formulaire de recherche : tous les filtres (fond,
// source, dates, facettes Légifrance/JUDILIBRE…), taxonomies JUDILIBRE
// dynamiques, exécution de la recherche (doSearch, routée selon le fond vers
// /rechercher, /bocc-texts-list ou les endpoints de liste des fonds
// "spéciaux"), historique de requêtes, pagination.
//
// Autonome : ne dépend d'aucun état externe au hook, à l'exception de
// `facetsOpen` (état d'affichage du panneau de facettes, resté dans
// JuridiquePanel.tsx car purement UI) qui conditionne le préchargement des
// taxonomies JUDILIBRE.
//
// Point de couplage documenté avec useDocumentViewer : `resultsFond` /
// `resultsSource` (fond/source figés au moment de LA recherche qui a produit
// `results`) sont passés par le JSX de JuridiquePanel.tsx à `openDocument`
// lors d'un clic sur un résultat — mais ce hook n'a lui-même aucune
// connaissance de useDocumentViewer.
//
// Extrait de JuridiquePanel.tsx (étape 5 du refactor — cf. discussion du
// 05/07/2026).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { useState, useCallback, useEffect, type FormEvent } from 'react';
import { API_BASE } from '../../../constants';
import { safeSetJSON } from '../storage';
import {
  fondRequiresQuery,
  normalizeBoccTextResult,
  normalizeDebatResult,
  normalizeQuestionResult,
  normalizeDocAdminResult,
} from '../search-helpers';
import {
  FONDS_SANS_RECHERCHE_TEXTUELLE,
  JURISDICTIONS_JUDILIBRE, PUBLICATIONS_JUDILIBRE, SOLUTIONS_JUDILIBRE,
} from '../constants';
import type { SearchResult, SearchResponse, Source, Fond } from '../types';

const PAGE_SIZE = 10;

/** Construit la borne "DD/MM/YYYY > DD/MM/YYYY" attendue par les endpoints
 *  de liste Légifrance (BOCC/débats/questions écrites) à partir des dates
 *  ISO (YYYY-MM-DD) des sélecteurs. Borne ouverte si un seul côté est fourni. */
function toIntervalPublication(start: string, end: string): string | undefined {
  if (!start && !end) return undefined;
  const fmt = (iso: string): string => {
    const [y, m, d] = iso.split('-');
    return `${d}/${m}/${y}`;
  };
  const s = start ? fmt(start) : '01/01/1900';
  const e = end ? fmt(end) : fmt(new Date().toISOString().slice(0, 10));
  return `${s} > ${e}`;
}

/** Fusionne une liste statique (libellés français soignés, prioritaires)
 *  avec les entrées dynamiques de la taxonomie absentes de la liste
 *  statique (par valeur) — purement additif, ne peut jamais retirer une
 *  option déjà proposée. */
function mergeOptions(
  staticList: { value: string; label: string }[],
  dynamicList: { value: string; label: string }[]
): { value: string; label: string }[] {
  const known = new Set(staticList.map((o) => o.value));
  const extra = dynamicList.filter((o) => !known.has(o.value));
  return [...staticList, ...extra];
}

export function useJuridiqueSearch(facetsOpen: boolean) {
  // ── État formulaire ─────────────────────────────────────────────────────────
  const [source, setSource] = useState<Source>('legifrance');
  const [fond, setFond] = useState<Fond>('LODA_ETAT');
  const [query, setQuery] = useState('');
  const [champRecherche, setChampRecherche] = useState('ALL');
  const [dateStart, setDateStart] = useState('');
  const [dateEnd, setDateEnd] = useState('');
  const [sort, setSort] = useState('PERTINENCE');
  const [jurisdiction, setJurisdiction] = useState<string[]>([]);
  const [publication, setPublication] = useState<string[]>([]);
  const [solution, setSolution] = useState<string[]>([]);
  // ── Nouveaux filtres Légifrance ──────────────────────────────────────────
  const [nature, setNature] = useState<string[]>([]);
  const [legalStatus, setLegalStatus] = useState<string[]>([]);
  // ── Nouveaux filtres JUDILIBRE ───────────────────────────────────────────
  const [chamber, setChamber] = useState<string[]>([]);
  const [formation, setFormation] = useState<string[]>([]);
  const [theme, setTheme] = useState<string[]>([]);
  const [themeInput, setThemeInput] = useState('');
  const [location, setLocation] = useState<string[]>([]);
  const [locationInput, setLocationInput] = useState('');
  const [judField, setJudField] = useState<string[]>([]);
  const [judOperator, setJudOperator] = useState('or');
  const [withFileOfType, setWithFileOfType] = useState<string[]>([]);
  const [particularInterest, setParticularInterest] = useState(false);
  // Taxonomies JUDILIBRE chargées dynamiquement (GET /taxonomie/:id) —
  // complètent/valident les libellés statiques ci-dessus. Clé = id taxonomie
  // (+ éventuel contexte), valeur = liste {value,label} triée.
  const [taxonomies, setTaxonomies] = useState<Record<string, { value: string; label: string }[]>>({});
  const [taxonomiesLoading, setTaxonomiesLoading] = useState<Record<string, boolean>>({});
  // ── Filtres structurels des fonds spéciaux (BOCC / débats / questions
  // écrites / documents administratifs) — ces fonds n'ont pas de recherche
  // plein texte complète côté API, seulement ces critères. ─────────────────
  const [boccIdcc, setBoccIdcc] = useState('');
  const [typesPublication, setTypesPublication] = useState<string[]>([]); // débats
  const [parlementTypes, setParlementTypes] = useState<string[]>([]);     // questions écrites
  const [docAdminYears, setDocAdminYears] = useState('');                 // ex: "2022,2023"
  // Filtres génériques CNIL (facette_extra) — saisie libre : les valeurs
  // exactes de TYPE et NATURE_DELIB ne sont pas normalisées de façon fiable
  // dans la doc officielle (contrairement à LODA/JORF).
  const [cnilType, setCnilType] = useState<string[]>([]);
  const [cnilTypeInput, setCnilTypeInput] = useState('');
  const [cnilNatureDelib, setCnilNatureDelib] = useState<string[]>([]);
  const [cnilNatureDelibInput, setCnilNatureDelibInput] = useState('');
  const [page, setPage] = useState(1);

  // ── État résultats ──────────────────────────────────────────────────────────
  const [results, setResults] = useState<SearchResult[]>([]);
  // Fond / source figés au moment de la recherche qui a produit `results`.
  // Sert à étiqueter correctement chaque résultat affiché (kicker, extraction
  // d'id…) même si l'utilisateur change ensuite le filtre `fond`/`source`
  // sans relancer de recherche : sans cette capture, les résultats déjà
  // affichés changeaient rétroactivement d'identité (ex. un décret étiqueté
  // "CNIL" après un simple clic sur le filtre CNIL).
  const [resultsFond, setResultsFond] = useState<Fond>(fond);
  const [resultsSource, setResultsSource] = useState<Source>(source);
  const [total, setTotal] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasSearched, setHasSearched] = useState(false);

  // ── Historique ───────────────────────────────────────────────────────────
  const [history, setHistory] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem('juridique_history') || '[]'); }
    catch { return []; }
  });

  // ── Taxonomies JUDILIBRE (chargement dynamique) ─────────────────────────
  // Alimente les facettes chambre/formation/matière/siège avec les valeurs
  // réelles de l'API (GET /taxonomie/:id) plutôt que des listes figées —
  // ces nomenclatures évoluent et sont trop nombreuses (matière : ~450
  // entrées) pour être codées en dur de façon fiable. Repli silencieux :
  // en cas d'échec, les listes/inputs restent utilisables (saisie libre
  // pour matière/siège, message d'indisponibilité pour chambre/formation).
  const fetchTaxonomy = useCallback(async (id: string, contextValue?: string) => {
    const cacheKey = `${id}:${contextValue || ''}`;
    if (taxonomies[cacheKey] || taxonomiesLoading[cacheKey]) return;
    setTaxonomiesLoading((prev) => ({ ...prev, [cacheKey]: true }));
    try {
      const qs = contextValue ? `?context_value=${encodeURIComponent(contextValue)}` : '';
      const res = await fetch(`${API_BASE}/api/juridique/taxonomie/${encodeURIComponent(id)}${qs}`);
      const data = await res.json();
      // Forme officielle attendue : { result: [{key,value}, ...] }. La forme
      // exacte pour une requête "liste complète" (sans `key`/`value`) n'est
      // pas totalement figée dans le swagger DILA (le schéma documente le
      // cas d'une clé unique) : on gère donc défensivement plusieurs formes.
      let items: { value: string; label: string }[] = [];
      const resultField = (data && typeof data === 'object') ? (data as Record<string, unknown>).result : undefined;
      if (Array.isArray(resultField)) {
        items = (resultField as Array<Record<string, unknown>>)
          .map((it) => ({ value: String(it.key ?? ''), label: String(it.value ?? it.key ?? '') }))
          .filter((it) => it.value);
      } else if (resultField && typeof resultField === 'object') {
        const it = resultField as Record<string, unknown>;
        if (it.key) items = [{ value: String(it.key), label: String(it.value ?? it.key) }];
      } else if (Array.isArray(data)) {
        items = (data as Array<Record<string, unknown>>)
          .map((it) => ({ value: String(it.key ?? it.value ?? ''), label: String(it.value ?? it.key ?? '') }))
          .filter((it) => it.value);
      }
      items.sort((a, b) => a.label.localeCompare(b.label, 'fr'));
      setTaxonomies((prev) => ({ ...prev, [cacheKey]: items }));
    } catch {
      // Silencieux : l'UI retombe sur les valeurs statiques / la saisie libre.
    } finally {
      setTaxonomiesLoading((prev) => ({ ...prev, [cacheKey]: false }));
    }
  }, [taxonomies, taxonomiesLoading]);

  // Chambre/formation dépendent du contexte juridiction (cc par défaut, ca
  // sinon — seules valeurs supportées par l'API pour ces deux taxonomies).
  const taxoContext = jurisdiction.includes('ca') && !jurisdiction.includes('cc') ? 'ca' : 'cc';
  useEffect(() => {
    if (source !== 'judilibre' || !facetsOpen) return;
    fetchTaxonomy('chamber', taxoContext);
    fetchTaxonomy('formation', taxoContext);
    // Matière et siège : nomenclatures indépendantes du contexte de chambre.
    fetchTaxonomy('theme');
    fetchTaxonomy('location', 'ca');
    fetchTaxonomy('location', 'tj');
    // Nature de décision (arret, qpc, ordonnance, saisie, avis…) — partage
    // le même champ d'état `nature` que le filtre Légifrance équivalent
    // (le backend route ce champ vers `type` côté JUDILIBRE, cf. routes.rs).
    fetchTaxonomy('type');
    // Juridiction / publication / solution ont déjà des listes statiques
    // complètes d'après le swagger, mais celui-ci les qualifie de "etc." —
    // on charge la taxonomie en complément pour ne rien manquer, sans
    // perdre les libellés français déjà soignés (fusion, voir mergeOptions).
    fetchTaxonomy('jurisdiction');
    fetchTaxonomy('publication');
    fetchTaxonomy('solution');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, facetsOpen, taxoContext]);

  const chamberOptions = taxonomies[`chamber:${taxoContext}`] || [];
  const formationOptions = taxonomies[`formation:${taxoContext}`] || [];
  const themeOptions = taxonomies['theme:'] || [];
  const locationOptions = [...(taxonomies['location:ca'] || []), ...(taxonomies['location:tj'] || [])];
  const decisionTypeOptions = taxonomies['type:'] || [];
  const jurisdictionOptions = mergeOptions(JURISDICTIONS_JUDILIBRE, taxonomies['jurisdiction:'] || []);
  const publicationOptions = mergeOptions(PUBLICATIONS_JUDILIBRE, taxonomies['publication:'] || []);
  const solutionOptions = mergeOptions(SOLUTIONS_JUDILIBRE, taxonomies['solution:'] || []);

  const doSearch = useCallback(async (targetPage = 1) => {
    const needsQuery = fondRequiresQuery(source, fond);
    if (needsQuery && !query.trim()) return;
    setLoading(true);
    setError(null);
    setHasSearched(true);
    // Fige le fond/source de CETTE recherche : les résultats affichés doivent
    // garder leur identité même si l'utilisateur clique ensuite sur un autre
    // filtre sans relancer de recherche.
    setResultsFond(fond);
    setResultsSource(source);

    // ── Fonds "spéciaux" Légifrance sans capacité de recherche /search ─────
    // (BOCC / débats parlementaires / questions écrites / documents
    // administratifs) : routés vers leurs propres endpoints de liste, dont
    // la forme de réponse diffère de SearchResponse (clé `texts` pour BOCC,
    // filtres purement structurels pour les 3 autres — cf. normalizeXxx).
    if (source === 'legifrance' && FONDS_SANS_RECHERCHE_TEXTUELLE.includes(fond)) {
      // DEBAT / QUESTION_ECRITE / DOC_ADMIN
      try {
        let endpoint = '';
        let payload: Record<string, unknown> = { page: targetPage, page_size: PAGE_SIZE };
        if (fond === 'DEBAT') {
          endpoint = 'debats-list';
          if (typesPublication.length) payload.types_publication = typesPublication;
          const iv = toIntervalPublication(dateStart, dateEnd);
          if (iv) payload.date_parution = iv;
          payload.sort = sort !== 'PERTINENCE' ? sort : 'DEBAT_PARLEMENTAIRE_DESC';
        } else if (fond === 'QUESTION_ECRITE') {
          endpoint = 'questions-ecrites-list';
          if (parlementTypes.length) payload.parlement_types = parlementTypes;
          const iv = toIntervalPublication(dateStart, dateEnd);
          if (iv) payload.periode_publication = iv;
          payload.sort = sort !== 'PERTINENCE' ? sort : 'QUESTION_ECRITE_PARLEMENTAIRE_DESC';
        } else {
          endpoint = 'docs-admins';
          const years = docAdminYears
            .split(/[,\s]+/).map((y) => parseInt(y, 10)).filter((y) => !isNaN(y) && y > 1900 && y < 2200);
          payload = years.length ? { years } : {};
        }
        const res = await fetch(`${API_BASE}/api/juridique/${endpoint}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        const data = await res.json();
        if (data.error) throw new Error(data.error);
        const rawResults: Record<string, unknown>[] = data.results || [];
        const normalized = rawResults.map(
          fond === 'DEBAT' ? normalizeDebatResult
            : fond === 'QUESTION_ECRITE' ? normalizeQuestionResult
            : normalizeDocAdminResult
        );
        // Filtrage client léger par titre : ces fonds n'ont pas de recherche
        // plein texte côté API, mais on affine la page reçue si l'utilisateur
        // a tout de même saisi un terme (meilleur effort, non exhaustif).
        const filtered = query.trim()
          ? normalized.filter((r) => (r.title || '').toLowerCase().includes(query.trim().toLowerCase()))
          : normalized;

        if (fond === 'DOC_ADMIN') {
          // `/list/docsAdmins` (DocsAdminsListRequest = { years } uniquement)
          // ne supporte AUCUNE pagination côté serveur : l'API renvoie
          // systématiquement l'intégralité des résultats pour les années
          // demandées. On pagine donc côté client pour que Suivant/Précédent
          // affichent réellement des pages différentes plutôt que de
          // réafficher la même liste complète à chaque clic.
          const start = (targetPage - 1) * PAGE_SIZE;
          setResults(filtered.slice(start, start + PAGE_SIZE));
          setTotal(filtered.length);
        } else {
          setResults(filtered);
          setTotal(data.totalResultNumber ?? normalized.length ?? null);
        }
        setPage(targetPage);
      } catch (e: unknown) {
        setError((e as Error).message || 'Erreur de recherche');
        setResults([]);
      } finally {
        setLoading(false);
      }
      return;
    }

    if (source === 'legifrance' && fond === 'BOCC') {
      try {
        const payload: Record<string, unknown> = { page: targetPage, page_size: PAGE_SIZE };
        if (query.trim()) payload.titre = query.trim();
        if (boccIdcc.trim()) payload.idcc = boccIdcc.trim();
        const iv = toIntervalPublication(dateStart, dateEnd);
        if (iv) payload.interval_publication = iv;
        payload.sort = sort !== 'PERTINENCE' ? sort : 'BOCC_SORT_DESC';
        const res = await fetch(`${API_BASE}/api/juridique/bocc-texts-list`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        const data = await res.json();
        if (data.error) throw new Error(data.error);
        const rawTexts: Record<string, unknown>[] = data.texts || [];
        setResults(rawTexts.map(normalizeBoccTextResult));
        setTotal(data.totalResultNumber ?? rawTexts.length ?? null);
        setPage(targetPage);
      } catch (e: unknown) {
        setError((e as Error).message || 'Erreur de recherche');
        setResults([]);
      } finally {
        setLoading(false);
      }
      return;
    }

    // ── Recherche standard (/rechercher) : tous les autres fonds ───────────
    const body: Record<string, unknown> = {
      source,
      query,
      page: targetPage,
      page_size: PAGE_SIZE,
    };

    if (source === 'legifrance') {
      body.fond = fond;
      body.champ_recherche = champRecherche !== 'ALL' ? champRecherche : undefined;
      body.sort = sort;
      if (nature.length) body.nature = nature;
      if (legalStatus.length) body.legal_status = legalStatus;
      if (fond === 'CNIL') {
        const extra: { facette: string; valeurs: string[] }[] = [];
        if (cnilType.length) extra.push({ facette: 'TYPE', valeurs: cnilType });
        if (cnilNatureDelib.length) extra.push({ facette: 'NATURE_DELIB', valeurs: cnilNatureDelib });
        if (extra.length) body.facette_extra = extra;
      }
      if (dateStart) body.date_start = dateStart;
      if (dateEnd) body.date_end = dateEnd;
    } else {
      body.operator = judOperator;
      body.batch = targetPage - 1;
      body.batch_size = PAGE_SIZE;
      body.sort = sort === 'PERTINENCE' ? 'scorepub' : sort;
      body.resolve_references = true;
      if (judField.length) body.field = judField;
      if (jurisdiction.length) body.jurisdiction = jurisdiction;
      if (chamber.length) body.chamber = chamber;
      if (formation.length) body.formation = formation;
      if (theme.length) body.theme = theme;
      if (location.length) body.location = location;
      if (publication.length) body.publication = publication;
      if (solution.length) body.solution = solution;
      if (withFileOfType.length) body.with_file_of_type = withFileOfType;
      if (particularInterest) body.particular_interest = true;
      if (dateStart) body.date_start = dateStart;
      if (dateEnd) body.date_end = dateEnd;
    }

    try {
      const res = await fetch(`${API_BASE}/api/juridique/rechercher`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data: SearchResponse = await res.json();
      if (data.error) throw new Error(data.error);

      setResults(data.results || []);
      setTotal(data.total ?? data.totalResultNumber ?? null);
      setPage(targetPage);

      if (query.trim()) {
        const newHistory = [query, ...history.filter((h) => h !== query)].slice(0, 15);
        setHistory(newHistory);
        safeSetJSON('juridique_history', newHistory);
      }
    } catch (e: unknown) {
      setError((e as Error).message || 'Erreur de recherche');
      setResults([]);
    } finally {
      setLoading(false);
    }
  }, [
    source, fond, query, champRecherche, sort, dateStart, dateEnd, history,
    nature, legalStatus,
    jurisdiction, publication, solution, chamber, formation, theme, location,
    judField, judOperator, withFileOfType, particularInterest,
    boccIdcc, typesPublication, parlementTypes, docAdminYears,
    cnilType, cnilNatureDelib,
  ]);

  const handleSubmit = useCallback((e: FormEvent) => {
    e.preventDefault();
    doSearch(1);
  }, [doSearch]);

  // ── Recherche déclenchée depuis un lien de secours (rapprochement/texte
  // appliqué SANS id/url résolu par l'API — cf. extract-judilibre.ts) ────────
  // `doSearch` lit `query`/`source`/`fond` depuis l'état du hook (closure) :
  // on ne peut donc pas l'appeler juste après `setQuery`/`setSource` dans le
  // même tick (React n'a pas encore commité le nouvel état, `doSearch`
  // partirait avec les anciennes valeurs). On pose l'état puis on incrémente
  // un compteur ; un effet dédié déclenche `doSearch(1)` au rendu SUIVANT,
  // une fois l'état à jour et `doSearch` reconstruit avec les nouvelles
  // valeurs dans ses dépendances.
  const [autoSearchTick, setAutoSearchTick] = useState(0);
  const searchFor = useCallback((q: string, opts: { source: Source; fond?: Fond }) => {
    setQuery(q);
    setSource(opts.source);
    if (opts.fond) setFond(opts.fond);
    setAutoSearchTick((t) => t + 1);
  }, []);
  useEffect(() => {
    if (autoSearchTick === 0) return;
    doSearch(1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoSearchTick]);

  // ── Réinitialisation des filtres ────────────────────────────────────────────
  const resetFilters = useCallback(() => {
    setDateStart('');
    setDateEnd('');
    setSort('PERTINENCE');
    setJurisdiction([]);
    setPublication([]);
    setSolution([]);
    setChampRecherche('ALL');
    setNature([]);
    setLegalStatus([]);
    setChamber([]);
    setFormation([]);
    setTheme([]);
    setThemeInput('');
    setLocation([]);
    setLocationInput('');
    setJudField([]);
    setJudOperator('or');
    setWithFileOfType([]);
    setParticularInterest(false);
    setBoccIdcc('');
    setTypesPublication([]);
    setParlementTypes([]);
    setDocAdminYears('');
    setCnilType([]);
    setCnilTypeInput('');
    setCnilNatureDelib([]);
    setCnilNatureDelibInput('');
  }, []);

  const activeFilterCount =
    (dateStart ? 1 : 0) + (dateEnd ? 1 : 0) +
    (champRecherche !== 'ALL' ? 1 : 0) +
    jurisdiction.length + publication.length + solution.length +
    nature.length + legalStatus.length +
    chamber.length + formation.length + theme.length + location.length +
    judField.length + (judOperator !== 'or' ? 1 : 0) +
    withFileOfType.length + (particularInterest ? 1 : 0) +
    (boccIdcc.trim() ? 1 : 0) + typesPublication.length + parlementTypes.length +
    (docAdminYears.trim() ? 1 : 0) + cnilType.length + cnilNatureDelib.length;

  const totalPages = total != null ? Math.ceil(total / PAGE_SIZE) : null;

  return {
    // Formulaire
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
    // Résultats
    results, setResults, resultsFond, resultsSource, total, setTotal, loading, error, hasSearched,
    // Historique
    history, setHistory,
    // Taxonomies
    taxonomies, taxonomiesLoading, fetchTaxonomy, taxoContext,
    chamberOptions, formationOptions, themeOptions, locationOptions,
    decisionTypeOptions, jurisdictionOptions, publicationOptions, solutionOptions,
    // Actions
    doSearch, handleSubmit, resetFilters, activeFilterCount, totalPages,
    searchFor,
  };
}
