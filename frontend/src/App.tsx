// ============================================================================
// Juridix — Assistant IA desktop
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


import React, { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import './App.css';
import { DialogProvider, useDialog } from './DialogContext';
import { useSpaces, useConversations, useChatStream, useDraggable } from './hooks/index';
import type { Conversation, Space } from './hooks/index';

type LoadableConversation = { id: string; title: string; messages: ChatMessage[]; space_id?: string | null };
import { genId } from './utils/text';
import { hasArtifacts, extractArtifacts } from './utils/artifacts';
import type { CitationGrounding } from './utils/citations';
import { dossierToolsScope, mcpServersFor, officialMcpUrls, type ToolsScope } from './utils/officialMcp';
import { useVeilleBadge } from './utils/veille';
import { Message } from './components/Message';
import type { ChatMessage, Attachment } from './components/Message';
import { ArtifactsPanel } from './components/ArtifactsPanel';
import { SettingsModal, McpStatusPanel, AboutModal, McpTooltipButton, type McpEntry } from './components/Modals';
import type { Settings } from './components/Modals';
import { PromptsEditorModal } from './components/PromptsEditorModal';
import { IngestionModal } from './components/IngestionModal';
import { JuridiquePanel } from './components/JuridiquePanel';
import { DroitADateControl } from './components/DroitADateControl';
import { API_BASE, IMAGE_MIME, IMAGE_EXTS, DOC_EXTS, MIME_TO_EXT, HISTORY_ITEMS } from './constants';
import { getSecretStorageIssue, loadSecretsIntoSettings, onSecretStorageIssue, persistSecrets, stripSecrets } from './secureStorage';
import { McpAuthDialog } from './components/McpAuthDialog';
import { onMcpBlocked, serverHost, type McpBlocked } from './utils/mcpGrants';

const DEFAULT_SETTINGS: Settings = { endpoint: '', bearer: '', model: '', web_search_mcp: '', web_search_mcp_alias: '', mcp_servers: [], winnow_enabled: false };

// Applique immédiatement les préférences de police sur :root
// Appelé au démarrage ET à chaque changement dans les settings
function applyFontPrefs(fontFamily?: string, fontSize?: number) {
  const FONT_STACKS: Record<string, string> = {
    'dm-sans':   "'DM Sans', sans-serif",
    'inter':     "'Inter', sans-serif",
    'system-ui': "system-ui, -apple-system, sans-serif",
    'ubuntu':    "'Ubuntu', 'Cantarell', sans-serif",
  };
  const root = document.documentElement;
  root.style.setProperty('--font-body', FONT_STACKS[fontFamily ?? 'dm-sans'] ?? FONT_STACKS['dm-sans']);
  root.style.setProperty('--font-size-base', `${fontSize ?? 14}px`);
}

export default function App() {
  return (
    <DialogProvider>
      <AppInner />
    </DialogProvider>
  );
}

interface CurrentUser {
  name?: string;
  email?: string;
}

function AppInner() {
  const { toast: toastFn } = useDialog();
  const { spaces, loadingSpaces, reloadSpaces } = useSpaces();
  const { conversations, saveConversation, deleteConversation } = useConversations();

  const [settings, setSettings] = useState<Settings>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem('juridix_settings') || '{}');
      const s = { ...DEFAULT_SETTINGS, ...saved };
      // Migration : mcp_servers était string[], normalise en {url, alias}[]
      if (Array.isArray(s.mcp_servers)) {
        s.mcp_servers = s.mcp_servers.map((entry: unknown) =>
          typeof entry === 'string' ? { url: entry } : entry
        );
      }
      applyFontPrefs(s.font_family, s.font_size);
      return s;
    } catch { return DEFAULT_SETTINGS; }
  });
  const initCredentials = (s: Settings, email?: string) => {
    const rawKey = s.qdrant_api_key?.trim() || null;

    // Si la clé est un JWT utilisateur (commence par "eyJ"), tenter de décoder
    // le payload base64url pour extraire les collections autorisées et les passer
    // comme qdrant_collection_tokens — sans vérifier la signature (lecture seule).
    let collectionTokens: { collection: string; token: string }[] = [];
    if (rawKey && rawKey.startsWith('eyJ')) {
      try {
        const parts = rawKey.split('.');
        if (parts.length >= 2) {
          // base64url → base64 standard → atob
          const pad = (s: string) => s + '='.repeat((4 - s.length % 4) % 4);
          const payload = JSON.parse(atob(pad(parts[1].replace(/-/g, '+').replace(/_/g, '/'))));
          if (Array.isArray(payload?.access)) {
            // JWT scopé à plusieurs collections : { collection, access }[]
            collectionTokens = payload.access
              .filter((a: any) => typeof a?.collection === 'string')
              .map((a: any) => ({ collection: a.collection as string, token: rawKey }));
          } else if (typeof payload?.access === 'string') {
            // JWT global (access: "r" ou "rw") — pas de liste de collections
            collectionTokens = [];
          }
        }
      } catch { /* JWT illisible — on laisse le backend gérer */ }
    }

    return (window as any).__TAURI__?.core?.invoke('init_credentials', {
      payload: {
        endpoint:                 s.endpoint,
        bearer:                   s.bearer,
        qdrant_api_key:           rawKey,
        qdrant_url:               s.qdrant_url?.trim() || null,
        user_email:               email ?? currentUser?.email ?? null,
        qdrant_collection_tokens: collectionTokens,
        piste_client_id:          s.piste_client_id?.trim() || null,
        piste_client_secret:      s.piste_client_secret?.trim() || null,
      }
    })?.catch(console.error);
  };

  const saveSettings = (s: Settings) => {
    applyFontPrefs(s.font_family, s.font_size);
    setSettings(s);
    // Les trois champs sensibles (bearer, qdrant_api_key, piste_client_secret)
    // ne sont jamais écrits ici : `persistSecrets` les envoie au trousseau
    // système (ou au repli localStorage dédié hors app desktop) — voir
    // secureStorage.ts. `stripSecrets` garantit qu'aucun résidu en clair ne
    // subsiste dans le blob `juridix_settings`, y compris si l'utilisateur
    // vient de modifier un champ non sensible sans toucher aux secrets.
    localStorage.setItem('juridix_settings', JSON.stringify(stripSecrets(s)));
    persistSecrets(s).catch((e) => console.error('[secrets] persistSecrets a échoué', e));
    initCredentials(s);
  };

  const [showSettings,      setShowSettings]      = useState(!settings.endpoint);

  // Initialise AppState Rust dès le démarrage via IPC Tauri (pas HTTP).
  // On attend la résolution de la Promise avant d'autoriser les fetches HTTP
  // qui dépendent du bearer token (modèles, utilisateur…).
  // En parallèle, le bearer est aussi envoyé directement via le header X-Albert-Token
  // sur ces fetches, ce qui élimine toute dépendance au timing de l'IPC.
  const [credentialsReady, setCredentialsReady] = useState(false);
  useEffect(() => {
    (async () => {
      // Les secrets (bearer, qdrant_api_key, piste_client_secret) vivent
      // désormais dans le trousseau système, pas dans le `settings` chargé
      // synchroniquement depuis localStorage au tout premier rendu : il faut
      // les rapatrier ici — et migrer d'éventuelles valeurs héritées d'une
      // installation antérieure à ce chantier — AVANT d'appeler
      // `initCredentials`, sous peine d'envoyer des identifiants vides au
      // tout premier démarrage. Voir secureStorage.ts.
      const merged = await loadSecretsIntoSettings(settings);
      setSettings(merged);
      // Si une migration a eu lieu, purger immédiatement le blob localStorage
      // des valeurs en clair qu'il contenait encore.
      localStorage.setItem('juridix_settings', JSON.stringify(stripSecrets(merged)));

      const p = initCredentials(merged);
      if (p) p.then(() => setCredentialsReady(true)).catch(() => setCredentialsReady(true));
      else setCredentialsReady(true);
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const [showMcpStatus,     setShowMcpStatus]      = useState(false);
  const [showIngestion,     setShowIngestion]      = useState(false);
  const [showPromptsEditor, setShowPromptsEditor]  = useState(false);
  const [showAbout,         setShowAbout]          = useState(false);
  // Trousseau système défaillant : les secrets ne sont pas enregistrés sur disque (voir secureStorage.ts).
  const [secretIssue, setSecretIssue] = useState<string | null>(getSecretStorageIssue());
  useEffect(() => onSecretStorageIssue(setSecretIssue), []);
  // Serveurs MCP écartés par le backend faute d'autorisation valide (R-02) : avertissement non
  // bloquant ; l'utilisateur examine puis autorise d'un seul geste (tous les outils du serveur).
  const [mcpBlocked, setMcpBlocked] = useState<McpBlocked[]>([]);
  const [mcpAuth, setMcpAuth] = useState<{ server: string; alias?: string } | null>(null);
  const [mcpGrantMsg, setMcpGrantMsg] = useState<string | null>(null);
  useEffect(() => onMcpBlocked(list => setMcpBlocked(prev => {
    const m = new Map(prev.map(b => [b.server, b] as const));
    for (const b of list) m.set(b.server, b);
    return [...m.values()];
  })), []);
  useEffect(() => {
    if (!mcpGrantMsg) return;
    const t = setTimeout(() => setMcpGrantMsg(null), 9000);
    return () => clearTimeout(t);
  }, [mcpGrantMsg]);
  const mcpAliasFor = (server: string): string | undefined =>
    settings.mcp_servers?.find(x => x.url === server)?.alias
    || (server === settings.web_search_mcp ? settings.web_search_mcp_alias || undefined : undefined);
  // viewMode pilote l'affichage du panneau juridique à gauche du chat.
  // 'juridique' est la vue par défaut de Juridix.
  const [viewMode, setViewMode] = useState<'juridique' | 'chat'>('juridique');
  // Compat : showJuridique reste utilisé par le menu Tauri ; piloté désormais par viewMode.
  const showJuridique = viewMode === 'juridique';
  const setShowJuridique = useCallback((v: boolean | ((prev: boolean) => boolean)) => {
    setViewMode(prev => {
      const prevBool = prev === 'juridique';
      const next = typeof v === 'function' ? (v as (p: boolean) => boolean)(prevBool) : v;
      return next ? 'juridique' : 'chat';
    });
  }, []);

  const [spaceAssignments, setSpaceAssignments] = useState<Record<string, string | null>>(() => {
    try {
      const raw: Record<string, string | null> = JSON.parse(localStorage.getItem('juridix_space_assignments_v2') || '{}');
      // Nettoyer les valeurs null explicites — l'absence de clé suffit pour "non assigné"
      return Object.fromEntries(Object.entries(raw).filter(([, v]) => v !== null));
    } catch { return {}; }
  });
  const [collectionNames, setCollectionNames] = useState<Record<string, string>>(() => {
    try { return JSON.parse(localStorage.getItem('juridix_collection_names') || '{}'); } catch { return {}; }
  });
  const reloadAssignments = () => {
    try {
      const raw: Record<string, string | null> = JSON.parse(localStorage.getItem('juridix_space_assignments_v2') || '{}');
      setSpaceAssignments(Object.fromEntries(Object.entries(raw).filter(([, v]) => v !== null)));
    } catch { setSpaceAssignments({}); }
    try { setCollectionNames(JSON.parse(localStorage.getItem('juridix_collection_names') || '{}')); } catch { setCollectionNames({}); }
  };

  const [messages,      setMessages]      = useState<ChatMessage[]>([]);
  const inputRef = useRef<HTMLTextAreaElement>(null); // non-contrôlé : évite de re-rendre App sur chaque frappe
  const { dispatchChat: streamChat, loading, stopStreaming, lastUsage } = useChatStream(
    setMessages as React.Dispatch<React.SetStateAction<import('./hooks/index').ChatStreamMessage[]>>,
  );
  const [activeSpaceId, setActiveSpaceId] = useState<string | null>(null);
  const [attachments,   setAttachments]   = useState<Attachment[]>([]);
  const [extracting,    setExtracting]    = useState(false);
  const [webSearch,     setWebSearch]     = useState(false);
  const [artifactsMsgIndex, setArtifactsMsgIndex] = useState<number | null>(null);

  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [modelMaxCtx,     setModelMaxCtx]     = useState<Record<string, number>>({});
  const [activeModel,     setActiveModel]     = useState('');
  const [modelDropOpen,   setModelDropOpen]   = useState(false);
  const modelDropRef = useRef<HTMLDivElement>(null);

  const [sidebarOpen,   setSidebarOpen]   = useState(true);
  const [convSearch,    setConvSearch]    = useState('');
  const [currentUser,   setCurrentUser]   = useState<CurrentUser | null>(null);
  const [inputDragOver, setInputDragOver] = useState(false);

  const [currentConvId,    setCurrentConvId]    = useState(() => genId());
  const [currentConvTitle, setCurrentConvTitle] = useState<string | null>(null);
  const titleGeneratedRef = useRef(false);

  const chatEndRef          = useRef<HTMLDivElement>(null);
  const chatAreaRef         = useRef<HTMLDivElement>(null);
  const [showScrollBtn, setShowScrollBtn] = useState(false);
  const fileInputRef        = useRef<HTMLInputElement>(null);
  const currentConvIdRef    = useRef(currentConvId);
  const currentConvTitleRef = useRef(currentConvTitle);
  const activeSpaceRef      = useRef<Space | null>(null);
  const settingsRef         = useRef(settings);

  useEffect(() => { currentConvIdRef.current    = currentConvId;    }, [currentConvId]);
  useEffect(() => { currentConvTitleRef.current = currentConvTitle; }, [currentConvTitle]);
  useEffect(() => { settingsRef.current         = settings;         }, [settings]);
  const activeModelRef = useRef('');
  useEffect(() => { activeModelRef.current = activeModel; }, [activeModel]);

  const activeSpace = spaces.find(s => s.id === activeSpaceId) || spaces[0] || null;
  useEffect(() => { activeSpaceRef.current = activeSpace; }, [activeSpace]);

  // Le modèle n'est plus saisi dans les réglages : il se choisit dans la liste récupérée auprès de
  // l'endpoint. `settings.model` n'est conservé que comme préférence héritée d'une version antérieure.
  const configured    = !!(settings.endpoint && settings.bearer);
  const artifactsOpen = artifactsMsgIndex !== null;
  const quickPrompts  = (activeSpace as { prompts?: { label: string; icon: string; prompt: string }[] } | null)?.prompts || [];

  // ── Overlay chat/artéfacts (mode juridique uniquement) ──────────────────────
  // Le chat n'est plus une colonne permanente : c'est un panneau flottant
  // posé par-dessus le document/la liste de résultats, ouvert à la demande
  // via le bouton flottant (FAB) ou automatiquement dès qu'une action
  // explicite de l'utilisateur déclenche une réponse. Les Artéfacts
  // remplacent le contenu de cet overlay plutôt que d'ouvrir une zone à part.
  const [chatOverlayOpen, setChatOverlayOpen] = useState(false);

  // Permet de faire glisser l'overlay de chat à l'écran via son header.
  const { panelRef: chatOverlayRef, handleProps: chatOverlayDragHandle, resizeProps: chatOverlayResizeProps, panelStyle: chatOverlayDragStyle, resetPosition: resetChatOverlayPosition, dragging: chatOverlayDragging } = useDraggable();

  const openChatOverlay = useCallback(() => {
    setChatOverlayOpen(true);
  }, []);

  useEffect(() => {
    if (!loadingSpaces && spaces.length && activeSpaceId === null) setActiveSpaceId(spaces[0].id);
  }, [loadingSpaces, spaces, activeSpaceId]);

  // Intercepte tous les clics sur liens externes et les ouvre via le shell Tauri
  // (la webview Tauri n'ouvre pas les liens target="_blank" nativement)
  // Utilise window.__TAURI__ injecté globalement (withGlobalTauri: true dans tauri.conf.json)
  useEffect(() => {
    const handleLinkClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      const anchor = target.closest('a') as HTMLAnchorElement | null;
      if (!anchor) return;
      const href = anchor.getAttribute('href');
      if (!href || href.startsWith('#') || href.startsWith('javascript')) return;
      if (/^https?:\/\//.test(href) || /^mailto:/.test(href)) {
        e.preventDefault();
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const tauri = (window as any).__TAURI__;
        if (tauri?.shell?.open) {
          tauri.shell.open(href).catch(console.error);
        } else if (tauri?.opener?.open) {
          tauri.opener.open(href).catch(console.error);
        }
      }
    };
    document.addEventListener('click', handleLinkClick, true);
    return () => document.removeEventListener('click', handleLinkClick, true);
  }, []);

  useEffect(() => {
    const lastIdx = [...messages].map((m, i) => m.role === 'assistant' ? i : -1).filter(i => i >= 0).pop();
    if (lastIdx === undefined) return;
    const last = messages[lastIdx];
    if (last.streaming) return;
    if (hasArtifacts(typeof last.content === 'string' ? last.content : '')) {
      setArtifactsMsgIndex(lastIdx);
    }
  }, [messages, viewMode]);

  // Indicateur du FAB (point rouge) : dérivé directement de l'état de la
  // conversation plutôt que d'un booléen à maintenir manuellement — auparavant
  // (`hasUnseenResponse`) il ne s'allumait que si une réponse arrivait PENDANT
  // que l'overlay était fermé, pas si l'overlay était fermé APRÈS coup (ex :
  // l'utilisateur consulte la réponse via le panneau Artéfacts puis referme —
  // cf. capture d'écran utilisateur du 07/07). Ici, le point s'affiche dès
  // qu'il existe au moins une réponse de l'assistant ET que l'overlay est
  // fermé, quelle que soit la façon dont on en est arrivé là.
  const hasChatResponse = messages.some((m) => m.role === 'assistant' && !m.streaming);


  useEffect(() => {
    if (!modelDropOpen) return;
    const h = (e: MouseEvent) => {
      if (modelDropRef.current && !modelDropRef.current.contains(e.target as Node)) setModelDropOpen(false);
    };
    document.addEventListener('mousedown', h);
    return () => document.removeEventListener('mousedown', h);
  }, [modelDropOpen]);

  useEffect(() => {
    if (!credentialsReady || !settings.endpoint || !settings.bearer) return;

    const applyFallback = () => {
      if (settings.model) {
        setAvailableModels([settings.model]);
        setActiveModel(prev => prev || settings.model);
      }
    };

    fetch(`${API_BASE}/api-proxy/api/models?endpoint=${encodeURIComponent(settings.endpoint)}`, {
      headers: { 'X-Albert-Token': settings.bearer },
    })
      .then(r => r.ok ? r.json() : null)
      .then((data: { data?: { id: string; max_context_length?: number }[] } | null) => {
        const ids = data?.data?.map(m => m.id).filter(Boolean) || [];
        if (!ids.length) {
          // Liste vide (API inaccessible, 401 absorbé côté backend, ou endpoint sans /models) →
          // fallback sur le modèle par défaut configuré
          applyFallback();
          return;
        }
        setAvailableModels(ids);
        const ctxMap: Record<string, number> = {};
        data?.data?.forEach(m => { if (m.id && m.max_context_length) ctxMap[m.id] = m.max_context_length; });
        setModelMaxCtx(ctxMap);
        setActiveModel(prev => prev || settings.model || ids[0]);
      })
      .catch(applyFallback);
  }, [credentialsReady, settings.endpoint, settings.bearer, settings.model]);

  useEffect(() => {
    if (!credentialsReady || !settings.endpoint || !settings.bearer) { setCurrentUser(null); return; }
    fetch(`${API_BASE}/api-proxy/api/users/me?endpoint=${encodeURIComponent(settings.endpoint)}`, {
      headers: { 'X-Albert-Token': settings.bearer },
    })
      .then(r => r.ok ? r.json() : null)
      .then((user: CurrentUser | null) => {
        setCurrentUser(user);
        // Repousser les credentials avec l'email maintenant connu
        if (user?.email) initCredentials(settings, user.email);
      })
      .catch(() => setCurrentUser(null));
  }, [credentialsReady, settings.endpoint, settings.bearer]); // eslint-disable-line react-hooks/exhaustive-deps

  // Charge les collections Qdrant réelles et nettoie les assignments orphelins
  const refreshCollectionsAndAssignments = useCallback(async () => {
    try {
      const res = await fetch(`${API_BASE}/api-proxy/api/ingestion/collections?endpoint=qdrant`);
      if (!res.ok) return;
      const data: { data?: { id: string; name: string }[] } = await res.json();
      const existing = new Set((data.data || []).map(c => c.name));

      // Mettre à jour le cache de noms (id = name pour Qdrant)
      const cache: Record<string, string> = {};
      (data.data || []).forEach(c => { cache[c.name] = c.name; });
      localStorage.setItem('juridix_collection_names', JSON.stringify(cache));
      setCollectionNames(cache);

      // Purger les assignments qui pointent vers une collection supprimée
      setSpaceAssignments(prev => {
        const cleaned = Object.fromEntries(
          Object.entries(prev).filter(([, colName]) => colName !== null && existing.has(colName as string))
        ) as Record<string, string>;
        localStorage.setItem('juridix_space_assignments_v2', JSON.stringify(cleaned));
        return cleaned;
      });
    } catch { /* silencieux */ }
  }, []);

  useEffect(() => {
    if (!credentialsReady || !settings.endpoint || !settings.bearer) return;
    refreshCollectionsAndAssignments();
  }, [credentialsReady, settings.endpoint, settings.bearer, refreshCollectionsAndAssignments]);

  useEffect(() => {
    const handler = (e: CustomEvent<string>) => {
      const id = e.detail;
      if (id === 'new_conv')       newConversation();
      if (id === 'settings')       setShowSettings(true);
      if (id === 'spaces_editor')  setShowPromptsEditor(true);
      if (id === 'rag')            setShowIngestion(true);
      if (id === 'juridique')      setShowJuridique(true);
      if (id === 'about')          setShowAbout(true);
      if (id === 'toggle_sidebar') setSidebarOpen(v => !v);
    };
    window.addEventListener('tauri-menu', handler as EventListener);
    return () => window.removeEventListener('tauri-menu', handler as EventListener);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (messages.some(m => m.streaming)) chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // Affiche/masque le bouton "aller en bas"
  useEffect(() => {
    const el = chatAreaRef.current;
    if (!el) return;
    const onScroll = () => {
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
      setShowScrollBtn(distanceFromBottom > 120);
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    onScroll();
    return () => el.removeEventListener('scroll', onScroll);
  }, []);

  const scrollToBottom = () => chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });

  const prevStreamingRef = useRef(false);
  useEffect(() => {
    const isStreaming = messages.some(m => m.streaming);
    if (prevStreamingRef.current && !isStreaming)
      requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => chatEndRef.current?.scrollIntoView({ behavior: 'smooth' }), 50)));
    prevStreamingRef.current = isStreaming;
  }, [messages]);

  useEffect(() => {
    const isStreaming = messages.some(m => m.streaming);
    if (isStreaming || messages.length < 2) return;
    const lastMsg = messages[messages.length - 1];
    if (lastMsg?.role !== 'assistant' || lastMsg?.streaming) return;
    const doSave = async () => {
      const convId = currentConvIdRef.current;
      const space  = activeSpaceRef.current;
      const cfg    = settingsRef.current;
      let convTitle = currentConvTitleRef.current;
      const now = new Date().toISOString();
      if (!titleGeneratedRef.current) {
        titleGeneratedRef.current = true;
        const firstUser      = messages.find(m => m.role === 'user')?.content || '';
        const firstAssistant = messages.find(m => m.role === 'assistant')?.content || '';
        const firstUserStr   = typeof firstUser === 'string' ? firstUser : (firstUser as Array<{ text?: string }>)[0]?.text || '';
        const firstAssistStr = typeof firstAssistant === 'string' ? firstAssistant : '';

        // Fallback immédiat : 5 premiers mots propres, première lettre en majuscule
        const words         = firstUserStr.trim().replace(/[*_#`~]/g, '').split(/\s+/).filter(Boolean);
        const rawFallback   = words.slice(0, 5).join(' ');
        const fallbackTitle = rawFallback
          ? rawFallback.charAt(0).toUpperCase() + rawFallback.slice(1)
          : 'Conversation';

        // 1. Sauvegarder immédiatement avec le fallback pour que la sidebar soit à jour
        setCurrentConvTitle(fallbackTitle);
        convTitle = fallbackTitle;
        const serializedFast = messages.map(m => ({
          role: m.role, content: m.content,
          ...(m.displayContent ? { displayContent: m.displayContent } : {}),
          ...(m.attachments    ? { attachments: m.attachments }       : {}),
        }));
        await saveConversation({ id: convId, title: fallbackTitle, space_id: space?.id || null, messages: serializedFast, created_at: now, updated_at: now });

        // 2. Améliorer avec le titre LLM en arrière-plan
        const titleModel = activeModelRef.current || cfg.model;
        if (cfg.endpoint && cfg.bearer && titleModel) {
          try {
            const res = await fetch(`${API_BASE}/api-proxy/api/generate-title`, {
              method: 'POST', headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ first_user: firstUserStr.slice(0, 400), first_assistant: firstAssistStr.slice(0, 400), model: titleModel }),
            });
            if (res.ok) {
              const generated = (await res.json()).title?.trim() || '';
              if (generated && generated !== 'Conversation') {
                convTitle = generated;
                setCurrentConvTitle(generated);
                // Mettre à jour la sauvegarde avec le titre définitif
                await saveConversation({ id: convId, title: generated, space_id: space?.id || null, messages: serializedFast, created_at: now, updated_at: now });
              }
            }
          } catch { /* on garde le fallback */ }
        }
        return; // sauvegarde déjà faite ci-dessus
      }
      const serialized = messages.map(m => ({
        role: m.role, content: m.content,
        ...(m.displayContent ? { displayContent: m.displayContent } : {}),
        ...(m.attachments    ? { attachments: m.attachments }       : {}),
      }));
      await saveConversation({ id: convId, title: convTitle || 'Conversation', space_id: space?.id || null, messages: serialized, created_at: now, updated_at: now });
    };
    doSave();
  }, [messages]); // eslint-disable-line

  const autoResize = () => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 120) + 'px';
  };

  const imageFileToAttachment = (file: File): Promise<Attachment> => new Promise((resolve, reject) => {
    const ext = MIME_TO_EXT[file.type] || 'png';
    const reader = new FileReader();
    reader.onload  = () => resolve({ filename: file.name || `image-collée.${ext}`, ext, type: 'image', dataUrl: reader.result as string });
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });

  const addImageAttachments = async (files: File[]) => {
    const images = files.filter(f => IMAGE_MIME.includes(f.type));
    if (!images.length) return;
    try {
      const newAtts = await Promise.all(images.map(imageFileToAttachment));
      setAttachments(prev => { const existing = new Set(prev.map(a => a.filename)); return [...prev, ...newAtts.filter(a => !existing.has(a.filename))]; });
    } catch { toastFn("Impossible de charger l'image", 'error'); }
  };

  const handlePaste = useCallback(async (e: React.ClipboardEvent) => {
    const items = Array.from(e.clipboardData?.items || []).filter(it => IMAGE_MIME.includes(it.type));
    if (!items.length) return;
    e.preventDefault();
    await addImageAttachments(items.map(it => it.getAsFile()).filter(Boolean) as File[]);
  }, []);

  const handleInputDragOver = useCallback((e: React.DragEvent) => {
    if (!Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; setInputDragOver(true);
  }, []);
  const handleInputDragLeave = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    if (e.currentTarget.contains(e.relatedTarget as Node)) return; setInputDragOver(false);
  }, []);
  const handleInputDrop = useCallback(async (e: React.DragEvent) => {
    e.preventDefault(); setInputDragOver(false);
    const files      = Array.from(e.dataTransfer.files);
    const imageFiles = files.filter(f => IMAGE_MIME.includes(f.type));
    const docFiles   = files.filter(f => DOC_EXTS.includes(f.name.toLowerCase().split('.').pop() || ''));
    if (imageFiles.length) await addImageAttachments(imageFiles);
    if (docFiles.length) {
      const dt = new DataTransfer(); docFiles.forEach(f => dt.items.add(f));
      if (fileInputRef.current) { fileInputRef.current.files = dt.files; fileInputRef.current.dispatchEvent(new Event('change', { bubbles: true })); }
    }
  }, []);

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const selectedFiles = Array.from(e.target.files || []);
    if (fileInputRef.current) fileInputRef.current.value = '';
    if (!selectedFiles.length) return;
    const getExt  = (f: File) => f.name.toLowerCase().split('.').pop() || '';
    const invalid = selectedFiles.filter(f => !([...IMAGE_EXTS, ...DOC_EXTS].includes(getExt(f))));
    if (invalid.length) { toastFn(`Fichiers non supportés : ${invalid.map(f => f.name).join(', ')}`, 'error'); return; }
    const images = selectedFiles.filter(f => IMAGE_EXTS.includes(getExt(f)));
    const docs   = selectedFiles.filter(f => DOC_EXTS.includes(getExt(f)));
    setExtracting(true);
    try {
      const imageAttachments = await Promise.all(images.map(f => new Promise<Attachment>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload  = () => resolve({ filename: f.name, ext: getExt(f), type: 'image', dataUrl: reader.result as string });
        reader.onerror = reject;
        reader.readAsDataURL(f);
      })));
      let docAttachments: Attachment[] = [];
      if (docs.length) {
        const formData = new FormData(); docs.forEach(f => formData.append('files', f));
        const res = await fetch(`${API_BASE}/api-proxy/api/extract-multiple`, { method: 'POST', body: formData });
        if (!res.ok) { const err = await res.json().catch(() => ({ detail: res.statusText })); toastFn(`Erreur : ${err.detail || res.statusText}`, 'error'); return; }
        const data: { errors?: { filename: string; error: string }[]; files?: Attachment[] } = await res.json();
        if (data.errors?.length) { const msgs = data.errors.map(e => `• ${e.filename} : ${e.error}`).join('\n'); if (!data.files?.length) { toastFn(`Erreurs :\n${msgs}`, 'error'); return; } toastFn(`Avertissement :\n${msgs}`, 'error'); }
        docAttachments = data.files || [];
      }
      setAttachments(prev => { const existing = new Set(prev.map(a => a.filename)); return [...prev, ...imageAttachments.filter(a => !existing.has(a.filename)), ...docAttachments.filter(a => !existing.has(a.filename))]; });
    } catch (err) { toastFn(`Erreur lors du chargement : ${(err as Error).message}`, 'error'); }
    finally { setExtracting(false); }
  };

  const _dispatchChat = useCallback(async ({ historyForApi, toolsScope }: { historyForApi: { role: string; content: unknown }[]; toolsScope?: ToolsScope }) => {
    const assignments   = (() => { try { return JSON.parse(localStorage.getItem('juridix_space_assignments_v2') || '{}'); } catch { return {}; } })();
    const assignedColId: string | null = activeSpace?.id ? (assignments[activeSpace.id] ?? null) : null;

    // Avertissement préventif : JWT sans URL Qdrant = RAG silencieusement cassé
    if (assignedColId && (settings.qdrant_api_key ?? '').startsWith('eyJ') && !settings.qdrant_url?.trim()) {
      toastFn("RAG inactif : configurez l'URL Qdrant dans les paramètres (obligatoire avec un token JWT).", 'error');
    }
    const baseModel     = activeModel || settings.model;
    if (!baseModel) {
      toastFn("Aucun modèle disponible : vérifiez l'endpoint et le token dans la configuration.", 'error');
      return null;
    }
    const lastUserText  = historyForApi.filter(m => m.role === 'user').pop()?.content || '';
    const lastUserStr   = typeof lastUserText === 'string' ? lastUserText : (lastUserText as Array<{ text?: string }>)[0]?.text || '';
    let modelForRequest = baseModel;
    if (/\b(word|docx|\.docx|document word|fichier word|rapport word)\b/i.test(lastUserStr) && /gpt/i.test(baseModel)) {
      const mistral = availableModels.find(m => /mistral/i.test(m));
      if (mistral) { modelForRequest = mistral; toastFn(`📄 Document Word — modèle basculé vers ${mistral}`, 'info'); }
    }
    return await streamChat({
      historyForApi,
      model: modelForRequest,
      spaceId: activeSpace?.id || null,
      // Analyse de dossier : aucun outil par défaut (restitution factuelle), ou
      // Légifrance/Judilibre seulement si l'utilisateur a coché « Compléter… ».
      mcpServers: mcpServersFor(toolsScope, settings.mcp_servers || [], settings.web_search_mcp, webSearch),
      collectionId: assignedColId,
    });
  }, [settings, activeModel, availableModels, activeSpace, webSearch, streamChat, toastFn]);

  const newConversation = () => {
    setMessages([]); if (inputRef.current) inputRef.current.value = ''; setAttachments([]); setArtifactsMsgIndex(null);
    setCurrentConvId(genId()); setCurrentConvTitle(null); titleGeneratedRef.current = false;
    if (inputRef.current) inputRef.current.style.height = 'auto';
  };
  const switchSpace = (spaceId: string) => { setActiveSpaceId(spaceId); newConversation(); };

  // Réaffiche un message déjà généré (relecture d'une analyse de dossier
  // archivée, cf. useDossiers.reopenDossierAnalysis) SANS appeler le LLM :
  // contrairement à `sendMessage`, le texte est injecté tel quel comme
  // réponse d'assistant plutôt qu'envoyé comme une nouvelle question — sinon
  // on gâcherait un appel LLM pour lui faire reformuler ce qu'il a déjà dit.
  const showStoredAnalysis = useCallback((message: string) => {
    setMessages([{ role: 'assistant', content: message }]);
    setCurrentConvId(genId()); setCurrentConvTitle(null); titleGeneratedRef.current = false;
    setArtifactsMsgIndex(null);
    if (viewMode === 'juridique') openChatOverlay();
  }, [viewMode, openChatOverlay]);

  const loadConversation = (conv: { id: string; title: string; messages: ChatMessage[]; space_id?: string | null }) => {
    setMessages(conv.messages || []); setCurrentConvId(conv.id); setCurrentConvTitle(conv.title);
    titleGeneratedRef.current = true; setArtifactsMsgIndex(null); if (inputRef.current) inputRef.current.value = ''; setAttachments([]);
    if (conv.space_id) setActiveSpaceId(conv.space_id);
    if (inputRef.current) inputRef.current.style.height = 'auto';
  };

  const sendMessage = useCallback(async (text?: string, opts?: { resetConversation?: boolean; completeWithMcp?: boolean }): Promise<string | null> => {
    const userText = (text || inputRef.current?.value || '').trim();
    if ((!userText && !attachments.length) || loading) return null;
    if (!configured) { setShowSettings(true); return null; }
    let fullContent: ChatMessage['content'], displayContent: string;
    let msgAttachments: Attachment[] | null = null;
    if (attachments.length) {
      const images = attachments.filter(a => a.type === 'image');
      const docs   = attachments.filter(a => a.type !== 'image');
      displayContent = userText || `Analyse de : ${attachments.map(a => a.filename).join(', ')}`;
      msgAttachments = attachments.map(a => ({ filename: a.filename, chars: a.chars, ext: a.ext, type: a.type }));
      if (images.length) {
        const parts: Array<{ type: string; text?: string; image_url?: { url: string } }> = [];
        const textPart = [userText || 'Analyse ces images.', ...docs.map(a => `--- Document : ${a.filename} ---\n\n${a.text}`)].join('\n\n');
        if (textPart) parts.push({ type: 'text', text: textPart });
        images.forEach(img => parts.push({ type: 'image_url', image_url: { url: img.dataUrl! } }));
        fullContent = parts;
      } else {
        fullContent = `${userText ? userText + '\n\n' : "Analyse ces documents et résume leur contenu.\n\n"}${docs.map(a => `--- Document : ${a.filename} ---\n\n${a.text}`).join('\n\n')}`;
      }
    } else { fullContent = userText; displayContent = userText; }
    if (viewMode === 'juridique') openChatOverlay();
    const userMsg: ChatMessage = { role: 'user', content: fullContent, displayContent, attachments: msgAttachments };
    // Une demande d'analyse de dossier (cf. onSendToChat côté JuridiquePanel)
    // démarre toujours une conversation neuve : l'historique envoyé au LLM ne
    // doit alors PAS inclure la conversation précédente. On ne peut pas se
    // contenter d'appeler `newConversation()` juste avant `sendMessage()` —
    // `messages` (capturé dans la closure de ce `useCallback`) resterait
    // l'ancien tableau jusqu'au prochain rendu, et le LLM recevrait quand même
    // tout l'historique précédent malgré un chat visuellement vidé. D'où ce
    // flag, qui court-circuite `messages` au lieu d'en dépendre.
    const baseHistory = opts?.resetConversation ? [] : messages;
    const historyForApi = [...baseHistory, { role: 'user', content: fullContent }];
    // Réponse produite à partir de documents fournis au modèle (analyse de dossier
    // ou pièces jointes) : ses citations seront confrontées à ces sources (« hors
    // sources » = vient de la mémoire du modèle). Un simple échange de chat n'a pas
    // de sources, donc pas d'ancrage.
    const grounded = !!opts?.resetConversation || attachments.some(a => a.type !== 'image');
    // `resetConversation` n'est utilisé que par les analyses de dossier : sans outils, sauf
    // si l'utilisateur a coché « Compléter via Légifrance / Judilibre ».
    const toolsScope: ToolsScope | undefined = opts?.resetConversation ? dossierToolsScope(!!opts.completeWithMcp) : undefined;
    if (opts?.resetConversation) {
      setMessages([userMsg, { role: 'assistant', content: '', streaming: true, grounded, toolsScope }]);
      setCurrentConvId(genId()); setCurrentConvTitle(null); titleGeneratedRef.current = false;
      setArtifactsMsgIndex(null);
    } else {
      setMessages(prev => [...prev, userMsg, { role: 'assistant', content: '', streaming: true, grounded, toolsScope }]);
    }
    if (inputRef.current) { inputRef.current.value = ''; autoResize(); } setAttachments([]);
    if (inputRef.current) inputRef.current.style.height = 'auto';
    return await _dispatchChat({ historyForApi, toolsScope });
  }, [messages, loading, attachments, configured, _dispatchChat, viewMode, openChatOverlay]);

  const regenerateMessage = useCallback(async (assistantMsgIndex: number) => {
    if (loading) return;
    const truncated = messages.slice(0, assistantMsgIndex);
    const toolsScope = messages[assistantMsgIndex]?.toolsScope;
    setMessages([...truncated, { role: 'assistant', content: '', streaming: true, grounded: messages[assistantMsgIndex]?.grounded, toolsScope }]);
    await _dispatchChat({ historyForApi: truncated.map(m => ({ role: m.role, content: m.content })), toolsScope });
  }, [messages, loading, _dispatchChat]);

  const editAndResend = useCallback(async (userMsgIndex: number, newText: string) => {
    if (loading || !newText.trim()) return;
    const before = messages.slice(0, userMsgIndex);
    setMessages([...before, { ...messages[userMsgIndex], content: newText, displayContent: newText }, { role: 'assistant', content: '', streaming: true, grounded: messages[userMsgIndex + 1]?.grounded, toolsScope: messages[userMsgIndex + 1]?.toolsScope }]);
    await _dispatchChat({ historyForApi: [...before.map(m => ({ role: m.role, content: m.content })), { role: 'user', content: newText }], toolsScope: messages[userMsgIndex + 1]?.toolsScope });
  }, [messages, loading, _dispatchChat]);

  const handleKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage(); }
  };

  const activeArtifactsContent = artifactsMsgIndex !== null ? messages[artifactsMsgIndex]?.content : null;
  const activeArtifacts = useMemo(
    () => typeof activeArtifactsContent === 'string' && activeArtifactsContent && artifactsMsgIndex !== null
      ? extractArtifacts(activeArtifactsContent, artifactsMsgIndex)
      : [],
    [activeArtifactsContent, artifactsMsgIndex],
  );
  const closeArtifacts = useCallback(() => setArtifactsMsgIndex(null), []);

  // Veille juridique : nombre de résultats non lus (relevé toutes les minutes) et signal
  // d'ouverture du panneau depuis le badge de la barre du haut.
  const { total: veilleUnread, refresh: refreshVeilleBadge } = useVeilleBadge();
  const [veilleSignal, setVeilleSignal] = useState(0);

  // Ancrage des citations : ce que le modèle a reçu pour répondre au message `msgIndex`
  // (textes des messages utilisateur précédents + clés des résultats d'outils/RAG de
  // ce tour). Lu via une ref : l'identité de la fonction reste stable, donc les
  // messages mémoïsés ne se re-rendent pas à chaque token.
  const messagesRef = useRef(messages);
  messagesRef.current = messages;
  const getGroundingFor = useCallback((msgIndex: number): CitationGrounding | null => {
    const all = messagesRef.current;
    const target = all[msgIndex];
    if (!target?.grounded) return null;
    const textOf = (c: ChatMessage['content']): string =>
      typeof c === 'string' ? c : c.map(p => p.text ?? '').filter(Boolean).join('\n');
    const sources = all.slice(0, msgIndex).filter(m => m.role === 'user').map(m => textOf(m.content)).filter(Boolean);
    return { sources, keys: target.groundingKeys ?? [] };
  }, []);
  // Question à laquelle répond un message, pour retrouver l'article qu'elle désigne (« Cite
  // l'article 1240… ») quand la réponse n'en cite aucun. Conversation libre seulement : pour une
  // analyse de documents, la « question » est un corpus entier et l'ancrage s'applique déjà.
  const getQuestionFor = useCallback((msgIndex: number): string | null => {
    const all = messagesRef.current;
    const target = all[msgIndex];
    if (!target || target.grounded) return null;
    for (let i = msgIndex - 1; i >= 0; i--) {
      if (all[i].role !== 'user') continue;
      const c = all[i].content;
      const text = typeof c === 'string' ? c : c.map(p => p.text ?? '').filter(Boolean).join('\n');
      return text.trim() ? text.slice(0, 2000) : null;
    }
    return null;
  }, []);
  const getArtifactsGrounding = useCallback(
    () => (artifactsMsgIndex === null ? null : getGroundingFor(artifactsMsgIndex)),
    [artifactsMsgIndex, getGroundingFor],
  );

  const lastArtifactsMsgIndex = (() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i];
      if (m.role === 'assistant' && !m.streaming && hasArtifacts(typeof m.content === 'string' ? m.content : '')) return i;
    }
    return null;
  })();

  const globalBadges = (
    <>
      {veilleUnread > 0 && (
        <button
          className="topbar-veille-btn"
          title={`${veilleUnread} nouveau(x) résultat(s) de veille juridique`}
          onClick={() => { setViewMode('juridique'); setVeilleSignal(n => n + 1); }}
        >
          🔔<span className="topbar-veille-btn__count">{veilleUnread}</span>
        </button>
      )}
      {lastUsage && (
        <div className="topbar-usage" title={`Tokens : ${lastUsage.promptTokens} in / ${lastUsage.completionTokens} out`}>
          <span className="usage-icon">🔢</span>
          <span className="usage-tokens">
            <span className="usage-in">{lastUsage.promptTokens.toLocaleString()}</span>
            <span className="usage-sep">→</span>
            <span className="usage-out">{lastUsage.completionTokens.toLocaleString()}</span>
          </span>
          {lastUsage.kgCO2eq != null && <span className="usage-co2">🌱 {lastUsage.kgCO2eq < 0.001 ? `${(lastUsage.kgCO2eq * 1e6).toFixed(1)} µgCO₂` : lastUsage.kgCO2eq < 1 ? `${(lastUsage.kgCO2eq * 1000).toFixed(2)} gCO₂` : `${lastUsage.kgCO2eq.toFixed(3)} kgCO₂`}</span>}
        </div>
      )}
      {lastArtifactsMsgIndex !== null && (
        <button
          className={`artifacts-toggle-btn ${artifactsOpen ? 'artifacts-toggle-btn--active' : ''}`}
          onClick={() => {
            if (artifactsOpen) { setArtifactsMsgIndex(null); }
            else {
              setArtifactsMsgIndex(lastArtifactsMsgIndex);
              if (viewMode === 'juridique') openChatOverlay();
            }
          }}
        >
          <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor"><path d="M2 2h5v5H2V2zm7 0h5v5H9V2zM2 9h5v5H2V9zm7 0h5v5H9V9z"/></svg>
          Artéfacts
        </button>
      )}
      {currentUser && <span className="badge badge-green" style={{ background: 'transparent', border: '1px solid var(--border)', color: 'var(--text-secondary)' }}>👤 {currentUser.name || currentUser.email}</span>}
      {configured ? (
        availableModels.length > 1 ? (
          <div className="model-pill-wrap" ref={modelDropRef}>
            <button
              className="model-pill-btn"
              aria-expanded={modelDropOpen}
              onClick={() => setModelDropOpen(v => !v)}
            >
              <span className="badge-dot" />
              {activeModel || settings.model}
              <svg className="model-pill-chevron" viewBox="0 0 16 16" fill="currentColor" width="9" height="9">
                <path d="M4 6l4 4 4-4" stroke="currentColor" strokeWidth="1.5" fill="none"/>
              </svg>
            </button>
            {modelDropOpen && (
              <div className="model-dropdown">
                {availableModels.map(m => (
                  <button
                    key={m}
                    type="button"
                    className={`model-dropdown-item${m === (activeModel || settings.model) ? ' model-dropdown-item--active' : ''}`}
                    onClick={() => { setActiveModel(m); setModelDropOpen(false); }}
                  >
                    {m === (activeModel || settings.model) ? '✓ ' : '\u00a0\u00a0'}{m}
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (activeModel || settings.model) ? (
          <span className="badge badge-green"><span className="badge-dot" />{activeModel || settings.model}</span>
        ) : (
          <span className="badge badge-red" onClick={() => setShowSettings(true)} style={{ cursor: 'pointer' }} title="La liste des modèles n'a pas pu être récupérée auprès de l'endpoint.">⚠ Aucun modèle disponible</span>
        )
      ) : (<span className="badge badge-red" onClick={() => setShowSettings(true)} style={{ cursor: 'pointer' }}>⚠ Non configuré</span>)}
    </>
  );

  return (
    <div className="layout">
      {(secretIssue || mcpBlocked.length > 0 || mcpGrantMsg) && (
        <div className="top-banners">
          {secretIssue && (
            <div className="secrets-banner" role="alert">
              <span>🔐 {secretIssue}</span>
              <button onClick={() => setSecretIssue(null)} aria-label="Fermer l'avertissement">✕</button>
            </div>
          )}
          {mcpBlocked.map(b => (
            <div key={b.server} className="secrets-banner" role="alert">
              <span>
                {b.status === 'changed'
                  ? `⚠️ Le serveur MCP « ${mcpAliasFor(b.server) || serverHost(b.server)} » a modifié ses outils depuis votre autorisation : ils n'ont pas été utilisés pour cette réponse.`
                  : `🔒 Le serveur MCP « ${mcpAliasFor(b.server) || serverHost(b.server)} » n'est pas autorisé : ses outils n'ont pas été utilisés pour cette réponse.`}
              </span>
              <span className="top-banners__actions">
                <button className="top-banners__btn" onClick={() => setMcpAuth({ server: b.server, alias: mcpAliasFor(b.server) })}>
                  Examiner et autoriser
                </button>
                <button onClick={() => setMcpBlocked(prev => prev.filter(x => x.server !== b.server))} aria-label="Ignorer">✕</button>
              </span>
            </div>
          ))}
          {mcpGrantMsg && (
            <div className="secrets-banner top-banners__ok" role="status">
              <span>✔ {mcpGrantMsg}</span>
              <button onClick={() => setMcpGrantMsg(null)} aria-label="Fermer">✕</button>
            </div>
          )}
        </div>
      )}
      {mcpAuth && (
        <McpAuthDialog
          server={mcpAuth.server}
          alias={mcpAuth.alias}
          onClose={(changed) => {
            const srv = mcpAuth.server;
            setMcpAuth(null);
            if (changed) {
              setMcpBlocked(prev => prev.filter(x => x.server !== srv));
              setMcpGrantMsg('Autorisation enregistrée. Posez de nouveau votre question pour que le modèle utilise ce serveur.');
            }
          }}
        />
      )}
      {showSettings      && <SettingsModal settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
      {showMcpStatus     && <McpStatusPanel servers={settings.mcp_servers || []} webSearchMcp={settings.web_search_mcp ? { url: settings.web_search_mcp, alias: settings.web_search_mcp_alias } : undefined} webSearchActive={webSearch} onClose={() => setShowMcpStatus(false)} />}
      {showIngestion     && <IngestionModal settings={settings} spaces={spaces} onClose={() => { setShowIngestion(false); refreshCollectionsAndAssignments(); }} onSave={(assignments) => { setSpaceAssignments(assignments); refreshCollectionsAndAssignments(); }} />}
      {showPromptsEditor && <PromptsEditorModal onClose={() => setShowPromptsEditor(false)} onSpacesUpdated={reloadSpaces} />}
      {showAbout         && <AboutModal onClose={() => setShowAbout(false)} />}
      {viewMode !== 'juridique' && (
        <aside className={`sidebar${sidebarOpen ? '' : ' sidebar--closed'}`}>
          <div className="sidebar-logo"><div className="brand-wrap"><div><div className="brand">Juridix</div><div className="tagline">Assistant juridique intelligent</div></div></div></div>
          <button className="new-conv-btn" onClick={newConversation}>
            <svg viewBox="0 0 16 16" fill="currentColor" width="13" height="13"><path d="M8 2a1 1 0 011 1v4h4a1 1 0 110 2H9v4a1 1 0 11-2 0V9H3a1 1 0 110-2h4V3a1 1 0 011-1z"/></svg>
            Nouvelle conversation
          </button>
          <div className="sidebar-section">Espaces</div>
          {loadingSpaces
            ? Array.from({ length: 5 }).map((_, i) => <div key={i} className="nav-item nav-item--skeleton" />)
            : spaces.map(space => (
                <div key={space.id} className={`nav-item ${activeSpace?.id === space.id ? 'active' : ''}`} onClick={() => switchSpace(space.id)}>
                  <span className="nav-icon">{(space as { icon?: string }).icon}</span>{(space as { label?: string }).label}
                  <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 4 }}>
                    {spaceAssignments[space.id] && <span className="rag-badge" title={`Collection : ${spaceAssignments[space.id]}`}>RAG</span>}
                    {(space as { dot?: string }).dot && <span className={`dot ${(space as { dot?: string }).dot}`} />}
                  </span>
                </div>
              ))}
          {HISTORY_ITEMS.length > 0 && (<><div className="sidebar-section" style={{ marginTop: 12 }}>Historique récent</div>{HISTORY_ITEMS.map((h, i) => (<div key={i} className="nav-item history"><span className="nav-icon">💬</span>{String(h)}</div>))}</>)}
          {conversations.length > 0 && (
            <>
              <div className="sidebar-section" style={{ marginTop: 12 }}>Conversations récentes</div>
              {conversations.length >= 8 && (
                <div className="conv-search-wrap">
                  <svg className="conv-search-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" width="13" height="13"><circle cx="6.5" cy="6.5" r="4.5"/><line x1="10.5" y1="10.5" x2="14" y2="14"/></svg>
                  <input className="conv-search-input" type="text" placeholder="Rechercher…" value={convSearch} onChange={e => setConvSearch(e.target.value)} spellCheck={false} />
                  {convSearch && <button className="conv-search-clear" onClick={() => setConvSearch('')}>✕</button>}
                </div>
              )}
              <div className="conv-list">
                {(conversations as Conversation[]).filter(conv => !convSearch || conv.title.toLowerCase().includes(convSearch.toLowerCase())).map(conv => (
                  <div key={conv.id} className={`conv-item ${conv.id === currentConvId ? 'conv-item--active' : ''}`} onClick={() => loadConversation(conv as LoadableConversation)}>
                    <span className="conv-icon">💬</span>
                    <span className="conv-title">{conv.title}</span>
                    <button className="conv-delete" onClick={e => { e.stopPropagation(); if (conv.id === currentConvId) newConversation(); deleteConversation(conv.id); }}>✕</button>
                  </div>
                ))}
                {convSearch && (conversations as Conversation[]).filter(c => c.title.toLowerCase().includes(convSearch.toLowerCase())).length === 0 && <div className="conv-no-result">Aucun résultat</div>}
              </div>
            </>
          )}
          <div className="sidebar-section" style={{ marginTop: 12 }}>Outils</div>
          <div
            className="nav-item"
            onClick={() => setViewMode('juridique')}
          >
            <span className="nav-icon">⚖️</span>
            Recherche juridique
          </div>
          <div
            className={`nav-item${viewMode === 'chat' ? ' active' : ''}`}
            onClick={() => setViewMode('chat')}
          >
            <span className="nav-icon">💬</span>
            Chat seul
          </div>
          <div className="sidebar-spacer" />
        </aside>
      )}

      <main className={`main${artifactsOpen ? ' main--with-artifacts' : ''}`}>
        {viewMode !== 'juridique' && (
        <header className="topbar">
          <div className="topbar-left">
            <button className="sidebar-toggle" onClick={() => setSidebarOpen(o => !o)}>
              <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z" clipRule="evenodd"/></svg>
            </button>
            {messages.length > 0 && (
              <button className="home-btn" onClick={newConversation}>
                <svg viewBox="0 0 20 20" fill="currentColor" width="15" height="15"><path d="M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h4a1 1 0 001-1v-3h2v3a1 1 0 001 1h4a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z"/></svg>
                Accueil
              </button>
            )}
            <div className="topbar-title">
              {currentConvTitle && <><span className="topbar-conv-title">{currentConvTitle}</span><span className="topbar-sep">·</span></>}
              {activeSpace ? `${(activeSpace as { icon?: string }).icon} ${(activeSpace as { label?: string }).label}` : 'Assistant RH'}
            </div>
          </div>
          <div className="topbar-badges">
            {globalBadges}
          </div>
        </header>
        )}

        <div className="chat-artifacts-wrapper">
          {viewMode === 'juridique' && (
            <div className="juridique-column juridique-column--full">
              <JuridiquePanel
                globalControls={globalBadges}
                winnowEnabled={!!settings.winnow_enabled}
                officialMcpAvailable={officialMcpUrls(settings.mcp_servers || []).length > 0}
                veilleSignal={veilleSignal}
                veilleUnread={veilleUnread}
                onVeilleChanged={refreshVeilleBadge}
                onSendToChat={async (message, opts) => {
                  // Une analyse de dossier (synthèse/points clés/contradictoire)
                  // porte sur un ensemble de documents précis, sans rapport
                  // avec une éventuelle conversation déjà en cours dans ce même
                  // chat flottant — on la démarre donc toujours dans une
                  // conversation neuve plutôt que de l'ajouter à la suite d'un
                  // échange sans lien, ce qui mélangeait les sujets. Passe par
                  // `resetConversation` (et non un `newConversation()` séparé
                  // suivi de `sendMessage()`) pour éviter que l'historique
                  // envoyé au LLM ne reste basé sur l'ancienne conversation le
                  // temps que React ne retombe à jour.
                  //
                  // On ATTEND ici la fin du streaming et on renvoie le texte
                  // final généré par le LLM à l'appelant (useDossiers) : c'est
                  // cette vraie réponse — pas le prompt envoyé — qui doit être
                  // conservée dans l'historique des analyses du dossier (cf.
                  // TODO B5bis du roadmap).
                  if (inputRef.current) {
                    inputRef.current.value = message;
                    inputRef.current.dispatchEvent(new Event('input', { bubbles: true }));
                  }
                  return await sendMessage(message, { resetConversation: true, completeWithMcp: !!opts?.completeWithMcp });
                }}
                onShowInChat={showStoredAnalysis}
                onClose={() => setViewMode('chat')}
              />
            </div>
          )}

          {/* ── Chat (mode "Chat seul") : colonne classique inchangée ──────── */}
          {viewMode !== 'juridique' && (
            <div className="chat-column">
              <div className="chat-area" ref={chatAreaRef}>
                {messages.length === 0 && (
                  <div className="welcome">
                    <div className="welcome-logo"><img src="/logo-juridix.png" alt="Juridix" className="welcome-logo-img" /></div>
                    <h2 className="welcome-title">
                      {activeSpace ? (activeSpace as { label?: string }).label : 'Bonjour\u00a0!'}
                    </h2>
                    <p className="welcome-sub">
                      {activeSpace ? 'Espace dédié — posez vos questions ou choisissez une suggestion ci-dessous.' : 'Votre assistant RH intelligent.'}<br />
                      <span className="welcome-attach-hint">📎 Joignez un PDF, Word ou image — ou collez une image directement (Ctrl+V).</span>
                    </p>
                    <div className="suggestion-grid">
                      {loadingSpaces
                        ? Array.from({ length: 6 }).map((_, i) => <div key={i} className="suggestion-card suggestion-card--skeleton" />)
                        : quickPrompts.map((q: { label: string; icon: string; prompt: string }) => (
                            <button key={q.label} className="suggestion-card" onClick={() => sendMessage(q.prompt)}>
                              <span className="suggestion-icon">{q.icon}</span>
                              <span className="suggestion-label">{q.label}</span>
                              <span className="suggestion-hint">{q.prompt.slice(0, 52)}…</span>
                            </button>
                          ))}
                    </div>
                  </div>
                )}
                {messages.map((msg, i) => (
                  <Message key={i} msg={msg} msgIndex={i}
                    onOpenArtifacts={setArtifactsMsgIndex}
                    onRegenerate={regenerateMessage}
                    onEditResend={editAndResend}
                    onStop={stopStreaming}
                    isLast={i === messages.length - 1}
                    getGrounding={getGroundingFor}
                    getQuestion={getQuestionFor}
                    loading={loading}
                  />
                ))}
                <div ref={chatEndRef} style={{ overflowAnchor: 'none', height: 1 }} />
              </div>

              {showScrollBtn && (
                <button className="scroll-to-bottom-btn" onClick={scrollToBottom} title="Aller en bas">
                  <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
                    <path d="M8 10.5L2.5 5h11L8 10.5z"/>
                  </svg>
                </button>
              )}

              <div className="input-zone">
                <input ref={fileInputRef} type="file" accept=".pdf,.docx,.doc,.png,.jpg,.jpeg,.gif,.webp" style={{ display: 'none' }} multiple onChange={handleFileChange} />
                <div className={`input-box ${loading ? 'input-box--loading' : ''} ${inputDragOver ? 'input-box--dragover' : ''}`} onDragOver={handleInputDragOver} onDragLeave={handleInputDragLeave} onDrop={handleInputDrop}>
                  {inputDragOver && (
                    <div className="input-drop-overlay">
                      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" strokeWidth="1.5"><path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5"/></svg>
                      Déposer l'image ici
                    </div>
                  )}
                  {attachments.length > 0 && (
                    <div className="input-attachments">
                      {attachments.map((att, i) => (
                        <div key={i} className={`input-attach-chip ${att.type === 'image' ? 'input-attach-chip--image' : ''}`}>
                          {att.type === 'image' ? (
                            <div className="attach-img-thumb-wrap">
                              <img src={att.dataUrl} alt={att.filename} className="attach-img-thumb" />
                              <div className="attach-img-preview"><img src={att.dataUrl} alt={att.filename} /></div>
                            </div>
                          ) : <span className="attach-icon">{att.ext === 'pdf' ? '📄' : '📝'}</span>}
                          <span className="attach-name">{att.filename}</span>
                          {att.type !== 'image' && att.chars != null && <span className="attach-chars">{(att.chars / 1000).toFixed(1)}k car.</span>}
                          <button className="attach-remove" onClick={() => setAttachments(prev => prev.filter((_, j) => j !== i))}>✕</button>
                        </div>
                      ))}
                    </div>
                  )}
                  <DroitADateControl />
                  <textarea ref={inputRef} className="chat-input"
                    placeholder={extracting ? 'Extraction en cours…' : attachments.length === 1 ? `Message à propos de "${attachments[0].filename}"…` : attachments.length > 1 ? `Message à propos de ${attachments.length} documents…` : configured ? `Posez votre question — ${(activeSpace as { label?: string })?.label || 'Assistant RH'}…` : "⚙ Configurez l'endpoint d'abord…"}
                    onChange={autoResize} onKeyDown={handleKey} onPaste={handlePaste} disabled={loading || extracting} rows={3}
                  />
                  <div className="input-toolbar">
                    <button className={`attach-btn ${extracting ? 'attach-btn--loading' : ''} ${attachments.length ? 'attach-btn--active' : ''}`} onClick={() => fileInputRef.current?.click()} disabled={loading || extracting}>
                      {extracting ? <span className="spinner-sm" /> : <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M8 4a3 3 0 00-3 3v4a5 5 0 0010 0V7a1 1 0 112 0v4a7 7 0 11-14 0V7a5 5 0 0110 0v4a3 3 0 11-6 0V7a1 1 0 012 0v4a1 1 0 102 0V7a3 3 0 00-3-3z" clipRule="evenodd"/></svg>}
                    </button>
                    <McpTooltipButton
                      className={`websearch-pill ${webSearch ? 'websearch-pill--on' : ''}`}
                      onClick={() => { setWebSearch(v => !v); if (!webSearch) setShowMcpStatus(true); }}
                      entries={settings.web_search_mcp ? [{ url: settings.web_search_mcp, alias: settings.web_search_mcp_alias || undefined }] : []}
                      label={<>
                        <svg viewBox="0 0 16 16" fill="currentColor" width="11" height="11"><path d="M8 1a7 7 0 100 14A7 7 0 008 1zM2.05 8.5h2.01c.06.93.22 1.8.46 2.55A5.01 5.01 0 012.05 8.5zm0-1h2.01A9.2 9.2 0 014.52 4.95 5.01 5.01 0 012.05 7.5zm5.45 5.97V11h-1.7c.22.75.52 1.36.85 1.78.27.35.57.55.85.55v.14zm0-3.47H5.07c-.06-.8-.1-1.62-.1-2.5h2.53v2.5zm0-3.5H4.97c0-.88.04-1.7.1-2.5H7.5V6.5zm0-3.47V1.14c-.28 0-.58.2-.85.55-.33.42-.63 1.03-.85 1.78h1.7v.14zM8.5 13.97V11h1.7c-.22.75-.52 1.36-.85 1.78-.27.35-.57.55-.85.55v.14zm0-3.47V8h2.53c0 .88-.04 1.7-.1 2.5H8.5zm0-3.5V4.5h2.43c.06.8.1 1.62.1 2.5H8.5zm0-3.47V1.14c.28 0 .58.2.85.55.33.42.63 1.03.85 1.78H8.5v.14z"/></svg>
                        {webSearch && settings.web_search_mcp_alias ? settings.web_search_mcp_alias : 'Web'}
                      </>}
                    />
                    {(settings.mcp_servers || []).length > 0 && (
                      <McpTooltipButton
                        className="websearch-pill websearch-pill--on mcp-active-pill"
                        onClick={() => setShowMcpStatus(true)}
                        entries={(settings.mcp_servers || []) as McpEntry[]}
                        label={<>
                          🔌 {(settings.mcp_servers || []).length === 1
                            ? ((settings.mcp_servers![0].alias) || `1 MCP`)
                            : `${settings.mcp_servers!.length} MCP`}
                        </>}
                      />
                    )}
                    <span className="input-hint">Shift+Entrée pour sauter une ligne</span>
                    <button className="send-btn" onClick={() => sendMessage()} disabled={loading || extracting || (!(inputRef.current?.value?.trim()) && !attachments.length)}>
                      {loading ? <span className="spinner" /> : <svg viewBox="0 0 16 16"><path d="M2 8l12-6-4 6 4 6z"/></svg>}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* ── Mode juridique : overlay chat/artéfacts flottant + FAB ──────────
             Le chat n'est plus une colonne permanente : il se pose par-dessus
             le document/la liste de résultats, à la demande. Les Artéfacts
             remplacent le contenu de cet overlay plutôt que d'ouvrir une 5ᵉ
             zone séparée. ──────────────────────────────────────────────────── */}
          {viewMode === 'juridique' && (
            <>
              <button
                className={`chat-fab${(hasChatResponse && !chatOverlayOpen) ? ' chat-fab--unseen' : ''}${chatOverlayOpen ? ' chat-fab--active' : ''}`}
                onClick={() => (chatOverlayOpen ? setChatOverlayOpen(false) : openChatOverlay())}
                title={chatOverlayOpen ? 'Fermer l\u2019assistant IA' : 'Ouvrir l\u2019assistant IA'}
                type="button"
              >
                {chatOverlayOpen ? (
                  <svg viewBox="0 0 16 16" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8"><path strokeLinecap="round" d="M4 4l8 8M12 4l-8 8"/></svg>
                ) : (
                  <svg viewBox="0 0 20 20" width="20" height="20" fill="currentColor"><path d="M2 4a2 2 0 012-2h12a2 2 0 012 2v8a2 2 0 01-2 2H7l-4 4v-4a2 2 0 01-1-1.73V4z"/></svg>
                )}
                {hasChatResponse && !chatOverlayOpen && <span className="chat-fab-dot" />}
              </button>

              {chatOverlayOpen && (
                <div
                  className={`chat-overlay${chatOverlayDragging ? ' chat-overlay--dragging' : ''}`}
                  ref={chatOverlayRef}
                  style={chatOverlayDragStyle}
                >
                  <div
                    className="chat-overlay-header"
                    {...chatOverlayDragHandle}
                    onDoubleClick={resetChatOverlayPosition}
                    title="Glisser pour déplacer · double-clic pour réinitialiser la position et la taille"
                  >
                    <span className="chat-overlay-title">
                      {artifactsOpen && activeArtifacts.length > 0 ? '✦ Artéfacts' : '💬 Assistant IA'}
                    </span>
                    <div className="chat-overlay-header-actions">
                      {artifactsOpen && activeArtifacts.length > 0 && (
                        <button className="jur-btn-link" onClick={closeArtifacts}>← Revenir au chat</button>
                      )}
                      <button className="jur-btn-icon jur-btn-icon--xs" title="Fermer" onClick={() => setChatOverlayOpen(false)}>✕</button>
                    </div>
                  </div>

                  {/* Poignées de redimensionnement : 4 bords et 4 coins. */}
                  {(['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'] as const).map(edge => (
                    <div key={edge} className={`chat-overlay-resize chat-overlay-resize--${edge}`} {...chatOverlayResizeProps(edge)} />
                  ))}

                  {/* Les Artéfacts remplacent le contenu de l'overlay (pas de
                     5ᵉ zone séparée) quand une réponse en contient. ────────── */}
                  {artifactsOpen && activeArtifacts.length > 0 ? (
                    <ArtifactsPanel artifacts={activeArtifacts} onClose={closeArtifacts} getGrounding={getArtifactsGrounding} fillParent />
                  ) : (
                    <>
                      <div className="chat-area chat-area--overlay" ref={chatAreaRef}>
                        {messages.length === 0 && (
                          <div className="welcome welcome--overlay">
                            <h2 className="welcome-title">Assistant juridique</h2>
                            <p className="welcome-sub">
                              Recherchez un texte ou une décision, ouvrez un dossier, puis lancez une synthèse — la réponse apparaît ici, avec ses artéfacts.
                            </p>
                          </div>
                        )}
                        {messages.map((msg, i) => (
                          <Message key={i} msg={msg} msgIndex={i}
                            onOpenArtifacts={setArtifactsMsgIndex}
                            onRegenerate={regenerateMessage}
                            onEditResend={editAndResend}
                            onStop={stopStreaming}
                            isLast={i === messages.length - 1}
                            getGrounding={getGroundingFor}
                            getQuestion={getQuestionFor}
                            loading={loading}
                          />
                        ))}
                        <div ref={chatEndRef} style={{ overflowAnchor: 'none', height: 1 }} />
                      </div>

                      <div className="input-zone">
                        <input ref={fileInputRef} type="file" accept=".pdf,.docx,.doc,.png,.jpg,.jpeg,.gif,.webp" style={{ display: 'none' }} multiple onChange={handleFileChange} />
                        <div className={`input-box ${loading ? 'input-box--loading' : ''} ${inputDragOver ? 'input-box--dragover' : ''}`} onDragOver={handleInputDragOver} onDragLeave={handleInputDragLeave} onDrop={handleInputDrop}>
                          {inputDragOver && (
                            <div className="input-drop-overlay">
                              <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" strokeWidth="1.5"><path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5"/></svg>
                              Déposer l'image ici
                            </div>
                          )}
                          {attachments.length > 0 && (
                            <div className="input-attachments">
                              {attachments.map((att, i) => (
                                <div key={i} className={`input-attach-chip ${att.type === 'image' ? 'input-attach-chip--image' : ''}`}>
                                  {att.type === 'image' ? (
                                    <div className="attach-img-thumb-wrap">
                                      <img src={att.dataUrl} alt={att.filename} className="attach-img-thumb" />
                                      <div className="attach-img-preview"><img src={att.dataUrl} alt={att.filename} /></div>
                                    </div>
                                  ) : <span className="attach-icon">{att.ext === 'pdf' ? '📄' : '📝'}</span>}
                                  <span className="attach-name">{att.filename}</span>
                                  {att.type !== 'image' && att.chars != null && <span className="attach-chars">{(att.chars / 1000).toFixed(1)}k car.</span>}
                                  <button className="attach-remove" onClick={() => setAttachments(prev => prev.filter((_, j) => j !== i))}>✕</button>
                                </div>
                              ))}
                            </div>
                          )}
                          <textarea ref={inputRef} className="chat-input"
                            placeholder={extracting ? 'Extraction en cours…' : attachments.length === 1 ? `Message à propos de "${attachments[0].filename}"…` : attachments.length > 1 ? `Message à propos de ${attachments.length} documents…` : configured ? 'Posez votre question — Assistant juridique…' : "⚙ Configurez l'endpoint d'abord…"}
                            onChange={autoResize} onKeyDown={handleKey} onPaste={handlePaste} disabled={loading || extracting} rows={1}
                          />
                          <div className="input-toolbar">
                            <button className={`attach-btn ${extracting ? 'attach-btn--loading' : ''} ${attachments.length ? 'attach-btn--active' : ''}`} onClick={() => fileInputRef.current?.click()} disabled={loading || extracting}>
                              {extracting ? <span className="spinner-sm" /> : <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M8 4a3 3 0 00-3 3v4a5 5 0 0010 0V7a1 1 0 112 0v4a7 7 0 11-14 0V7a5 5 0 0110 0v4a3 3 0 11-6 0V7a1 1 0 012 0v4a1 1 0 102 0V7a3 3 0 00-3-3z" clipRule="evenodd"/></svg>}
                            </button>
                            <McpTooltipButton
                              className={`websearch-pill ${webSearch ? 'websearch-pill--on' : ''}`}
                              onClick={() => { setWebSearch(v => !v); if (!webSearch) setShowMcpStatus(true); }}
                              entries={settings.web_search_mcp ? [{ url: settings.web_search_mcp, alias: settings.web_search_mcp_alias || undefined }] : []}
                              label={<>
                                <svg viewBox="0 0 16 16" fill="currentColor" width="11" height="11"><path d="M8 1a7 7 0 100 14A7 7 0 008 1zM2.05 8.5h2.01c.06.93.22 1.8.46 2.55A5.01 5.01 0 012.05 8.5zm0-1h2.01A9.2 9.2 0 014.52 4.95 5.01 5.01 0 012.05 7.5zm5.45 5.97V11h-1.7c.22.75.52 1.36.85 1.78.27.35.57.55.85.55v.14zm0-3.47H5.07c-.06-.8-.1-1.62-.1-2.5h2.53v2.5zm0-3.5H4.97c0-.88.04-1.7.1-2.5H7.5V6.5zm0-3.47V1.14c-.28 0-.58.2-.85.55-.33.42-.63 1.03-.85 1.78h1.7v.14zM8.5 13.97V11h1.7c-.22.75-.52 1.36-.85 1.78-.27.35-.57.55-.85.55v.14zm0-3.47V8h2.53c0 .88-.04 1.7-.1 2.5H8.5zm0-3.5V4.5h2.43c.06.8.1 1.62.1 2.5H8.5zm0-3.47V1.14c.28 0 .58.2.85.55.33.42.63 1.03.85 1.78H8.5v.14z"/></svg>
                                {webSearch && settings.web_search_mcp_alias ? settings.web_search_mcp_alias : 'Web'}
                              </>}
                            />
                            {(settings.mcp_servers || []).length > 0 && (
                              <McpTooltipButton
                                className="websearch-pill websearch-pill--on mcp-active-pill"
                                onClick={() => setShowMcpStatus(true)}
                                entries={(settings.mcp_servers || []) as McpEntry[]}
                                label={<>
                                  🔌 {(settings.mcp_servers || []).length === 1
                                    ? ((settings.mcp_servers![0].alias) || `1 MCP`)
                                    : `${settings.mcp_servers!.length} MCP`}
                                </>}
                              />
                            )}
                            <DroitADateControl />
                            <span className="input-hint">Shift+Entrée</span>
                            <button className="send-btn" onClick={() => sendMessage()} disabled={loading || extracting || (!(inputRef.current?.value?.trim()) && !attachments.length)}>
                              {loading ? <span className="spinner" /> : <svg viewBox="0 0 16 16"><path d="M2 8l12-6-4 6 4 6z"/></svg>}
                            </button>
                          </div>
                        </div>
                      </div>
                    </>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      </main>
    </div>
  );
}
