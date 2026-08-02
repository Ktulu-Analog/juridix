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


import { useState, useEffect, useCallback, useRef } from 'react';
import type React from 'react';
import { API_BASE } from '../constants';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Space {
  id: string;
  name: string;
  [key: string]: unknown;
}

export interface Conversation {
  id: string;
  title: string;
  updated_at: string;
  messages: unknown[];
  space_id?: string | null;
  /** Id du dossier juridique auquel ce fil est rattaché (cartable Juridix). */
  dossier_id?: string | null;
  created_at?: string;
  [key: string]: unknown;
}

// ── Hook : liste des espaces ──────────────────────────────────────────────────

export interface UseSpacesResult {
  spaces: Space[];
  loadingSpaces: boolean;
  reloadSpaces: () => void;
}

export function useSpaces(): UseSpacesResult {
  const [spaces, setSpaces]   = useState<Space[]>([]);
  const [loading, setLoading] = useState(true);
  const [tick, setTick]       = useState(0);
  const reload = () => setTick(t => t + 1);

  useEffect(() => {
    setLoading(true);
    fetch(`${API_BASE}/api-proxy/api/spaces`)
      .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
      .then((data: { spaces?: Space[] }) => setSpaces(data.spaces || []))
      .catch((err: unknown) => console.error('Impossible de charger les espaces :', err))
      .finally(() => setLoading(false));
  }, [tick]);

  return { spaces, loadingSpaces: loading, reloadSpaces: reload };
}

// ── Hook : gestion des conversations persistées ───────────────────────────────

export interface UseConversationsResult {
  conversations: Conversation[];
  saveConversation: (conv: Conversation) => Promise<void>;
  deleteConversation: (id: string) => Promise<void>;
  fetchConversations: () => Promise<void>;
}

export function useConversations(): UseConversationsResult {
  const [conversations, setConversations] = useState<Conversation[]>([]);

  const fetchConversations = useCallback(async () => {
    try {
      const res = await fetch(`${API_BASE}/api-proxy/api/conversations`);
      if (res.ok) {
        const data: { conversations?: Conversation[] } = await res.json();
        setConversations(data.conversations || []);
      }
    } catch { /* ignore */ }
  }, []);

  useEffect(() => { fetchConversations(); }, [fetchConversations]);

  const saveConversation = useCallback(async (conv: Conversation) => {
    try {
      await fetch(`${API_BASE}/api-proxy/api/conversations`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(conv),
      });
      setConversations(prev => {
        const idx = prev.findIndex(c => c.id === conv.id);
        const updated = idx >= 0 ? [...prev] : [conv, ...prev];
        if (idx >= 0) updated[idx] = conv;
        return updated.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      });
    } catch { /* ignore */ }
  }, []);

  const deleteConversation = useCallback(async (id: string) => {
    try {
      await fetch(`${API_BASE}/api-proxy/api/conversations/${id}`, { method: 'DELETE' });
      setConversations(prev => prev.filter(c => c.id !== id));
    } catch { /* ignore */ }
  }, []);

  return { conversations, saveConversation, deleteConversation, fetchConversations };
}

// ── Hook : streaming de chat réutilisable (extrait de App.tsx::_dispatchChat) ──
//
// Porte la logique SSE (deltas, rag_sources, mcp_image, usage) indépendamment
// du composant appelant : App.tsx (chat général) et JuridiquePanel.tsx (fil
// par dossier) partagent ce hook plutôt que de dupliquer le parsing SSE.

export interface ChatStreamMessage {
  role: 'user' | 'assistant';
  content: string | Array<{ type: string; text?: string; image_url?: { url: string } }>;
  displayContent?: string;
  streaming?: boolean;
  ragSources?: unknown[];
  [key: string]: unknown;
}

export interface ChatStreamUsage {
  promptTokens: number;
  completionTokens: number;
  kgCO2eq: number | null;
}

export interface ChatStreamRequestOptions {
  /** Historique complet à envoyer (le dernier message doit être l'user turn courant) */
  historyForApi: { role: string; content: unknown }[];
  model: string;
  spaceId?: string | null;
  mcpServers?: string[];
  collectionId?: string | null;
}

export interface UseChatStreamResult {
  /** Lance un tour de chat ; pousse les deltas dans setMessages au fil du stream.
   *  Résout avec le texte final assemblé de la réponse assistant (ou `null` en
   *  cas d'erreur/interruption) — permet aux appelants (ex : analyse de
   *  dossier juridique) de récupérer la VRAIE réponse du LLM plutôt que de
   *  devoir la relire depuis un state React potentiellement pas encore à jour. */
  dispatchChat: (opts: ChatStreamRequestOptions) => Promise<string | null>;
  loading: boolean;
  stopStreaming: () => void;
  lastUsage: ChatStreamUsage | null;
}

/**
 * @param setMessages état des messages géré par l'appelant (useState côté
 *   App.tsx, ou state local d'un fil de dossier dans JuridiquePanel).
 */
export function useChatStream(
  setMessages: React.Dispatch<React.SetStateAction<ChatStreamMessage[]>>,
): UseChatStreamResult {
  const [loading, setLoading] = useState(false);
  const [lastUsage, setLastUsage] = useState<ChatStreamUsage | null>(null);
  const abortControllerRef = useRef<AbortController | null>(null);

  const stopStreaming = useCallback(() => {
    if (abortControllerRef.current) { abortControllerRef.current.abort(); abortControllerRef.current = null; }
  }, []);

  const dispatchChat = useCallback(async (opts: ChatStreamRequestOptions) => {
    setLoading(true);
    // Accumulé en parallèle des `setMessages` (deltas de stream + image MCP)
    // pour pouvoir renvoyer le texte final assemblé à l'appelant sans devoir
    // relire un state React qui n'a pas forcément flush au moment du `await`.
    let finalContent = '';
    let ok = false;
    const controller = new AbortController(); abortControllerRef.current = controller;
    try {
      const res = await fetch(`${API_BASE}/api-proxy/api/chat`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: controller.signal,
        body: JSON.stringify({
          messages: opts.historyForApi,
          model: opts.model,
          stream: true,
          space_id: opts.spaceId ?? null,
          mcp_servers: opts.mcpServers || [],
          ...(opts.collectionId ? { collection_id: opts.collectionId } : {}),
        }),
      });
      if (!res.ok) { throw new Error(`Erreur ${res.status} : ${await res.text()}`); }
      const reader = res.body!.getReader(); const decoder = new TextDecoder();
      let tail = ''; let capturedUsage: ChatStreamUsage | null = null; let hasMcpImage = false;
      const dispatchSseBlock = (block: string) => {
        let eventType: string | null = null;
        let dataLines: string[] = [];
        for (const line of block.split('\n')) {
          if (line.startsWith('event:')) { eventType = line.slice(6).trim(); }
          else if (line.startsWith('data:'))  { dataLines.push(line.slice(5).trim()); }
        }
        const raw = dataLines.join('\n');
        if (!raw || raw === '[DONE]') return raw === '[DONE]' ? 'DONE' : null;
        if (eventType === 'rag_sources') {
          try { const p = JSON.parse(raw); if (p.sources) setMessages(prev => { const m = [...prev]; m[m.length-1] = { ...m[m.length-1], ragSources: p.sources }; return m; }); } catch { /* ignore */ }
          return null;
        }
        if (eventType === 'mcp_image') {
          try {
            const p = JSON.parse(raw);
            if (p.b64 && p.mime) {
              const bt = '```';
              const mdImg = bt + 'screenshot\n' + JSON.stringify({ mime: p.mime, alt: p.alt || 'Screenshot' }) + '\n' + p.b64 + '\n' + bt;
              hasMcpImage = true;
              finalContent += '\n\n' + mdImg;
              setMessages(prev => { const m = [...prev]; const last = m[m.length-1]; m[m.length-1] = { ...last, content: (typeof last.content === 'string' ? last.content : '') + '\n\n' + mdImg }; return m; });
            }
          } catch { /* ignore */ }
          return null;
        }
        try {
          const parsed = JSON.parse(raw); if (parsed.error) throw new Error(JSON.stringify(parsed.error));
          const delta = parsed.choices?.[0]?.delta?.content;
          if (delta && !hasMcpImage) { finalContent += delta; setMessages(prev => { const m = [...prev]; const last = m[m.length-1]; m[m.length-1] = { ...last, content: (typeof last.content === 'string' ? last.content : '') + delta, streaming: true }; return m; }); }
          if (parsed.usage) capturedUsage = { promptTokens: parsed.usage.prompt_tokens ?? 0, completionTokens: parsed.usage.completion_tokens ?? 0, kgCO2eq: parsed.usage.impacts?.kgCO2eq ?? null };
        } catch (e) {
          if (raw && raw !== '[DONE]') console.warn('[SSE] parse error on raw chunk:', (e as Error).message, '| raw[:120]:', raw.slice(0, 120));
        }
        return null;
      };
      outer: while (true) {
        const { done, value } = await reader.read();
        if (value) tail += decoder.decode(value, { stream: !done });
        if (done) break;
        let sepIdx: number;
        while ((sepIdx = tail.indexOf('\n\n')) !== -1) {
          const block = tail.slice(0, sepIdx);
          tail = tail.slice(sepIdx + 2);
          if (dispatchSseBlock(block) === 'DONE') break outer;
        }
      }
      if (capturedUsage) setLastUsage(capturedUsage);
      ok = true;
    } catch (err) {
      if ((err as Error).name !== 'AbortError') setMessages(prev => { const m = [...prev]; m[m.length-1] = { role: 'assistant', content: `❌ ${(err as Error).message}`, streaming: false }; return m; });
    } finally {
      abortControllerRef.current = null;
      setMessages(prev => { const m = [...prev]; const last = m[m.length-1]; if (last?.role === 'assistant') m[m.length-1] = { ...last, streaming: false }; return m; });
      setLoading(false);
    }
    return ok && finalContent ? finalContent : null;
  }, [setMessages]);

  return { dispatchChat, loading, stopStreaming, lastUsage };
}

// ============================================================================
// useDraggable — rend un panneau flottant déplaçable à la souris/au tactile
// via une « poignée » (ex : son header). Le panneau garde sa position par
// défaut (CSS right/bottom) tant que l'utilisateur ne l'a pas encore
// déplacé ; au premier drag, on bascule sur un positionnement libre
// (left/top en pixels), borné aux limites de la fenêtre.
// ============================================================================
export interface UseDraggableResult {
  /** À attacher à l'élément flottant (ex: la div .chat-overlay). */
  panelRef: React.RefObject<HTMLDivElement>;
  /** À attacher à la poignée de drag (ex: le header). */
  handleProps: {
    onMouseDown: (e: React.MouseEvent) => void;
    onTouchStart: (e: React.TouchEvent) => void;
    style: React.CSSProperties;
  };
  /** Style à fusionner sur l'élément flottant (position libre une fois déplacé). */
  panelStyle: React.CSSProperties;
  /** Remet le panneau à sa position CSS par défaut. */
  resetPosition: () => void;
  dragging: boolean;
}

export function useDraggable(): UseDraggableResult {
  const panelRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const dragState = useRef<{ offsetX: number; offsetY: number } | null>(null);

  const clampToViewport = useCallback((left: number, top: number) => {
    const el = panelRef.current;
    const w = el?.offsetWidth ?? 0;
    const h = el?.offsetHeight ?? 0;
    const maxLeft = Math.max(8, window.innerWidth - w - 8);
    const maxTop = Math.max(8, window.innerHeight - h - 8);
    return { left: Math.min(Math.max(8, left), maxLeft), top: Math.min(Math.max(8, top), maxTop) };
  }, []);

  const startDrag = useCallback((clientX: number, clientY: number) => {
    const el = panelRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    dragState.current = { offsetX: clientX - rect.left, offsetY: clientY - rect.top };
    setDragging(true);
  }, []);

  const moveDrag = useCallback((clientX: number, clientY: number) => {
    if (!dragState.current) return;
    const { offsetX, offsetY } = dragState.current;
    setPos(clampToViewport(clientX - offsetX, clientY - offsetY));
  }, [clampToViewport]);

  const endDrag = useCallback(() => {
    dragState.current = null;
    setDragging(false);
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const onMouseMove = (e: MouseEvent) => moveDrag(e.clientX, e.clientY);
    const onMouseUp = () => endDrag();
    const onTouchMove = (e: TouchEvent) => {
      const t = e.touches[0];
      if (t) { moveDrag(t.clientX, t.clientY); e.preventDefault(); }
    };
    const onTouchEnd = () => endDrag();
    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
    window.addEventListener('touchmove', onTouchMove, { passive: false });
    window.addEventListener('touchend', onTouchEnd);
    return () => {
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
      window.removeEventListener('touchmove', onTouchMove);
      window.removeEventListener('touchend', onTouchEnd);
    };
  }, [dragging, moveDrag, endDrag]);

  // Si la fenêtre est redimensionnée, on recadre la position pour rester visible.
  useEffect(() => {
    if (!pos) return;
    const onResize = () => setPos(p => p ? clampToViewport(p.left, p.top) : p);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [pos, clampToViewport]);

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    startDrag(e.clientX, e.clientY);
  }, [startDrag]);

  const onTouchStart = useCallback((e: React.TouchEvent) => {
    const t = e.touches[0];
    if (!t) return;
    startDrag(t.clientX, t.clientY);
  }, [startDrag]);

  const resetPosition = useCallback(() => setPos(null), []);

  const panelStyle: React.CSSProperties = pos
    ? { left: pos.left, top: pos.top, right: 'auto', bottom: 'auto' }
    : {};

  const handleProps = {
    onMouseDown,
    onTouchStart,
    style: { cursor: dragging ? 'grabbing' : 'grab', touchAction: 'none' as const },
  };

  return { panelRef, handleProps, panelStyle, resetPosition, dragging };
}
