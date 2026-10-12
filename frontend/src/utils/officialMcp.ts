// ============================================================================
// Juridix — Serveurs MCP officiels : quels outils pour quelle requête ?
// ============================================================================
// Une analyse de dossier (synthèse, contradictoire, points clés) est une
// « restitution factuelle » du dossier : par défaut elle n'a AUCUN outil. Si
// l'utilisateur coche « Compléter via Légifrance / Judilibre », seuls ces deux
// serveurs sont ajoutés — jamais la recherche web ni d'autres serveurs MCP.
// Un échange de chat ordinaire garde tous les outils configurés.

/** Portée des outils d'un message : `none` = aucun, `official` = Légifrance/Judilibre seulement.
 *  `undefined` = chat ordinaire (tous les outils configurés). */
export type ToolsScope = 'none' | 'official';

/** Identifiants des serveurs officiels (cf. mcp_stdio.rs::official_specs). */
const OFFICIAL_IDS = ['legifrance', 'judilibre'];

/** `stdio:mcp-legifrance`, `stdio:legifrance`, `stdio://legifrance`… (casse et espaces tolérés). */
export function isOfficialMcpUrl(url: string): boolean {
  const m = /^\s*stdio:\/*\s*(?:mcp-)?([a-z0-9_-]+)\s*\/*\s*$/i.exec(url);
  return !!m && OFFICIAL_IDS.includes(m[1].toLowerCase());
}

/** URLs des serveurs officiels présents dans la liste des réglages. */
export function officialMcpUrls(servers: { url: string }[]): string[] {
  return servers.map(s => s.url).filter(isOfficialMcpUrl);
}

/** Serveurs MCP à transmettre au backend de chat pour un message de portée `scope`. */
export function mcpServersFor(
  scope: ToolsScope | undefined,
  servers: { url: string }[],
  webSearchMcp?: string,
  webSearchActive?: boolean,
): string[] {
  if (scope === 'none') return [];
  if (scope === 'official') return officialMcpUrls(servers);
  return [...servers.map(s => s.url), ...(webSearchActive && webSearchMcp ? [webSearchMcp] : [])];
}

/** Portée d'outils d'une analyse de dossier selon la case cochée. */
export function dossierToolsScope(completeWithMcp: boolean): ToolsScope {
  return completeWithMcp ? 'official' : 'none';
}

// ── Sélection des outils exposés (Essentiels / Tous / Personnalisé) ───────────
// Chaque définition d'outil part avec CHAQUE requête de chat : l'écran affiche donc le
// coût en jetons de chaque choix avant que l'utilisateur ne le fasse.

export type ToolSelection =
  | { mode: 'essential' }
  | { mode: 'all' }
  | { mode: 'custom'; tools: string[] };

export interface CatalogTool {
  name: string;
  description: string;
  /** Coût estimé de la définition, en jetons par requête. */
  tokens: number;
  essential: boolean;
  exposed: boolean;
}

export interface ToolCatalog {
  tools: CatalogTool[];
  total: number;
  selection: ToolSelection;
  tokens_exposed: number;
}

/** « ≈ 5 900 jetons » (arrondi à la centaine au-delà de 1 000, pour ne pas afficher une fausse précision). */
export function formatTokens(n: number): string {
  const v = n >= 1000 ? Math.round(n / 100) * 100 : Math.round(n / 10) * 10;
  return `≈ ${v.toLocaleString('fr-FR').replace(/\u202f|\u00a0/g, ' ')} jetons`;
}

/** Outils effectivement exposés pour un mode donné (`custom` = noms cochés). */
export function toolsForMode(cat: ToolCatalog, mode: ToolSelection['mode'], custom: string[]): CatalogTool[] {
  if (mode === 'all') return cat.tools;
  if (mode === 'essential') return cat.tools.filter(t => t.essential);
  return cat.tools.filter(t => custom.includes(t.name));
}

export const sumTokens = (tools: CatalogTool[]) => tools.reduce((n, t) => n + t.tokens, 0);

/** Libellé court d'un serveur : « 6 sur 62 outils », « 62 outils », « 6 outils » (total inconnu). */
export function toolsLabel(s: { tools_total: number | null; tools_exposed: number | null }): string {
  const { tools_total: total, tools_exposed: exposed } = s;
  if (exposed == null) return total != null ? `${total} outils` : 'outils';
  if (total == null) return `${exposed} outil${exposed > 1 ? 's' : ''}`;
  return exposed === total ? `${total} outils` : `${exposed} sur ${total} outils`;
}
