// ============================================================================
// Juridix — Espace "Recherche juridique" — Helpers de mise en forme HTML/texte
// ============================================================================
// Fonctions PURES (aucune dépendance à React ni au state applicatif) : elles
// prennent des données brutes (réponses API, chaînes) et renvoient du
// HTML/texte prêt à afficher. Extraites de JuridiquePanel.tsx (étape 1 du
// refactor — cf. discussion du 04/07/2026) car ce sont précisément les
// fonctions les plus souvent réécrites au fil des corrections de bugs de
// mapping de champs API : les isoler ici les rend plus faciles à relire, à
// modifier sans risque de casser un JSDoc voisin, et à terme testables
// unitairement avec des fixtures JSON sans dépendre de l'API Légifrance en
// direct.
//
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import TurndownService from 'turndown';

// ── Dates ─────────────────────────────────────────────────────────────────────

/**
 * Formate une date renvoyée par l'API Légifrance, quel que soit son format
 * d'origine (epoch ms, epoch secondes, ISO...).
 *
 * La plupart des fonds Légifrance renvoient des MILLISECONDES depuis epoch,
 * mais certains (ex. Circulaire.dateSignature/dateDepot) renvoient des
 * SECONDES (10 chiffres, ex. 1423526400 → 10/02/2015). En dessous de 10^11
 * (~an 5138 en secondes / ~1973 en ms — hors de portée pour des textes
 * juridiques), on considère qu'il s'agit de secondes et on convertit ; au-
 * dessus, on traite comme des millisecondes.
 */
export function formatLegiDate(raw: unknown): string {
  if (!raw) return '';
  const opts: Intl.DateTimeFormatOptions = { day: '2-digit', month: 'long', year: 'numeric' };

  const n = Number(raw);
  if (!isNaN(n) && n > 0) {
    const ms = n < 100_000_000_000 ? n * 1000 : n;
    if (ms < 4_000_000_000_000) {
      const d = new Date(ms);
      if (!isNaN(d.getTime())) return d.toLocaleDateString('fr-FR', opts);
    }
  }

  if (typeof raw !== 'string' || !raw.trim()) return '';
  const s = raw.trim();

  // YYYY-MM-DD (sans time) — sûr sur tous les engines
  const ymd = s.match(/^(\d{4})-(\d{2})-(\d{2})/);
  if (ymd) {
    const d = new Date(Number(ymd[1]), Number(ymd[2]) - 1, Number(ymd[3]));
    if (!isNaN(d.getTime())) return d.toLocaleDateString('fr-FR', opts);
  }

  // Tentative générique (ISO avec timezone, etc.)
  const d = new Date(s);
  if (!isNaN(d.getTime())) return d.toLocaleDateString('fr-FR', opts);

  return '';
}

/** Identifie les préfixes d'id Légifrance qui désignent un TEXTE entier
 *  (par opposition à un article ou une section). */
export function isTextId(id: string): boolean {
  return /^(LEGITEXT|JORFTEXT|JURITEXT|KALITEXT|JORFCONT|CNILARTICLE_CONT|KALICONT|ACCOTEXTE)/.test(id);
}

// ── URLs de fichiers joints (PDF) ────────────────────────────────────────────

/**
 * Les fonds BOCC, débats parlementaires, questions écrites parlementaires et
 * documents administratifs ne renvoient PAS de contenu HTML/texte : ce sont
 * des fonds documentaires purement composés de fichiers PDF (parfois XML)
 * dont l'API ne fournit qu'un CHEMIN relatif (`pathFile`, `pathToFile`,
 * `attachmentUrl`…), jamais de contenu inline. C'est confirmé par le swagger
 * officiel : les schémas `EsTextBocc`, `Debat`, `EsQuestionsEcritesParlementaires`
 * et `DocumentAdministratif` n'ont aucun champ de type texte/HTML.
 *
 * Ces chemins sont relatifs au site public legifrance.gouv.fr (l'hôte de
 * l'API PISTE — api.piste.gouv.fr — sert uniquement le JSON, pas les
 * fichiers statiques). On les résout donc contre ce domaine.
 */
export function resolvePisteAssetUrl(path: unknown): string | null {
  if (typeof path !== 'string' || !path.trim()) return null;
  const p = path.trim();
  if (/^https?:\/\//i.test(p)) return p;
  return `https://www.legifrance.gouv.fr${p.startsWith('/') ? '' : '/'}${p}`;
}

/**
 * Construit l'URL de téléchargement PDF d'un débat parlementaire.
 *
 * Contrairement à BOCC/questions écrites/docs admin, dont `pathFile`/
 * `pathToFile` pointe vers un fichier statique servi tel quel par
 * www.legifrance.gouv.fr, les débats parlementaires sont servis par un
 * endpoint dédié qui prend `id`, `size` et `pathToFile` en query params :
 *   /download/pdf/debat?id=AN_2026-077.pdf&size=1,3 Mo&pathToFile=/debats/AN/20260612/AN_2026-077.pdf
 * (confirmé par observation directe du site public — absent du swagger, qui
 * ne documente que l'API JSON).
 *
 * ATTENTION : le paramètre `id` attendu ici n'est PAS `d.id` (identifiant
 * technique Légifrance, ex. `ANSETEXT000054301579`) mais le NOM DE FICHIER
 * du PDF (ex. `AN_2026-077.pdf`) — qui n'apparaît que comme dernier segment
 * de `pathToFile`. Utiliser `d.id` produit un lien qui télécharge le mauvais
 * fichier (ou échoue). On extrait donc le nom de fichier depuis `pathToFile`
 * plutôt que de faire confiance à `d.id`.
 */
export function resolveDebatPdfUrl(d: Record<string, unknown>): string | null {
  const pathToFile = (d.pathToFile as string) || '';
  if (!pathToFile.trim()) return null;
  const fileName = pathToFile.trim().split('/').filter(Boolean).pop() || '';
  const size = (d.displaySize as string) || '';
  const params = new URLSearchParams();
  if (fileName) params.set('id', fileName);
  if (size) params.set('size', size);
  params.set('pathToFile', pathToFile.trim());
  return `https://www.legifrance.gouv.fr/download/pdf/debat?${params.toString()}`;
}

/**
 * Construit l'URL de téléchargement PDF d'une question écrite parlementaire.
 *
 * Même défaut que celui corrigé ci-dessus pour les débats parlementaires :
 * `pathToFile` (ex. `/ANQUE_SENQUE/ANQUE/20260027/0001/p000/anq_20260027_
 * 0001_p000/anq_20260027_0001_p000.pdf`) n'est PAS directement servi tel
 * quel par www.legifrance.gouv.fr — passé en URL brute (résolution via
 * `resolvePisteAssetUrl`) le lien pointe vers une route inexistante. Le
 * fichier est en réalité servi par le même endpoint dédié `/download/pdf/`
 * que les débats, sous le slug `questionEcrite`, avec un paramètre
 * supplémentaire `title` absent côté débats (confirmé par observation
 * directe du site public le 08/07/2026) :
 *   /download/pdf/questionEcrite?id=anq_20260027_0001_p000.pdf&size=2,3 Mo
 *     &pathToFile=/ANQUE_SENQUE/ANQUE/.../anq_20260027_0001_p000.pdf
 *     &title=Année 2026 - N° 27 - Mardi 07 juillet 2026
 *
 * Comme pour les débats, `id` attendu ici est le NOM DE FICHIER (dernier
 * segment de `pathToFile`), pas `q.id` (identifiant technique Légifrance).
 * `title` sert de nom suggéré au téléchargement plutôt que de clé de
 * validation côté serveur : on le reconstruit du mieux possible à partir
 * des métadonnées disponibles (numéro et date de parution) sans viser une
 * réplique exacte du jour de la semaine DILA, non exposé par l'API.
 */
export function resolveQuestionEcritePdfUrl(q: Record<string, unknown>): string | null {
  const pathToFile = (q.pathToFile as string) || '';
  if (!pathToFile.trim()) return null;
  const fileName = pathToFile.trim().split('/').filter(Boolean).pop() || '';
  const size = (q.displaySize as string) || '';
  const numParution = (q.numeroParution as string) || '';
  const dateParution = formatLegiDate(q.dateParution);
  const title = [numParution ? `N° ${numParution}` : '', dateParution].filter(Boolean).join(' - ');
  const params = new URLSearchParams();
  if (fileName) params.set('id', fileName);
  if (size) params.set('size', size);
  params.set('pathToFile', pathToFile.trim());
  if (title) params.set('title', title);
  return `https://www.legifrance.gouv.fr/download/pdf/questionEcrite?${params.toString()}`;
}

/**
 * Construit le lien de téléchargement officiel d'une circulaire.
 * Le champ `Circulaire.attachmentUrl` (« Chemin vers le fichier PDF ») ne
 * correspond PAS à une route de téléchargement public valide — passé tel
 * quel il produit une URL invalide (ex. `/2015/04/cir_39431.pdf`, 404). Le
 * vrai point d'accès public, confirmé en le comparant à legifrance.gouv.fr,
 * est `/download/pdf/circ?id={id}` où `id` est l'identifiant de la
 * circulaire (`Circulaire.id`, ex. "39431") — pas un chemin de fichier.
 */
export function resolveCircPdfUrl(d: Record<string, unknown>): string | null {
  const id = (d.id as string) || '';
  if (!id.trim()) return null;
  return `https://www.legifrance.gouv.fr/download/pdf/circ?id=${encodeURIComponent(id.trim())}`;
}

// ── Texte brut → HTML ────────────────────────────────────────────────────────

function escapeText(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Repère soit une URL complète, soit un identifiant Légifrance nu, dans un
// paragraphe de texte BRUT (pas encore échappé).
const PLAIN_TEXT_LINK_PATTERN = /(https?:\/\/[^\s<>"]+)|(\b(?:LEGIARTI|JORFTEXT|JURITEXT|KALITEXT|LEGITEXT|KALICONT|JORFCONT)\d{12}\b)/g;

/**
 * Extrait un identifiant Légifrance interne (LEGIARTI…/LEGITEXT…) porté par
 * les paramètres `idArticle=`/`cidTexte=`/`textId=` d'une URL Légifrance
 * publique (ex. `affichCodeArticle.do?idArticle=LEGIARTI…`), ou `null` si
 * l'URL n'en contient pas. Partagé entre `linkifyPlainTextParagraph`
 * (renvois inline dans un texte brut) et la restitution du graphe de
 * citations JUDILIBRE (`textLink.url` du champ `visa`), qui pointe vers ces
 * mêmes URLs publiques Légifrance mais doit ouvrir l'article DANS
 * l'application plutôt que dans un nouvel onglet.
 */
export function resolveLegifranceInternalId(url: string): string | null {
  const m = url.match(/(?:idArticle|cidTexte|textId)=([A-Z]+\d{12})/);
  return m ? m[1] : null;
}

/**
 * Échappe un paragraphe de texte brut ET transforme au passage les URLs et
 * identifiants Légifrance qu'il contient en liens cliquables — sans ce
 * traitement, les « Texte(s) de référence » (URLs `affichCodeArticle.do?
 * idArticle=...`, `jo_pdf.jsp?...`) extraits d'une circulaire/d'un débat
 * s'affichaient en texte brut, non cliquables. Les URLs internes Légifrance
 * portant un `idArticle=`/`cidTexte=`/`textId=` deviennent des liens de
 * citation internes (mécanisme `data-cite-id` déjà utilisé ailleurs) ; les
 * autres URLs (ex. lien direct vers un PDF du JO) deviennent des liens
 * externes classiques.
 */
function linkifyPlainTextParagraph(paragraph: string): string {
  let html = '';
  let lastIndex = 0;
  for (const match of paragraph.matchAll(PLAIN_TEXT_LINK_PATTERN)) {
    const idx = match.index ?? 0;
    html += escapeText(paragraph.slice(lastIndex, idx));
    const [full, url, bareId] = match;
    if (url) {
      const internalId = resolveLegifranceInternalId(url);
      html += internalId
        ? `<a href="#" class="jur-citation jur-citation--id" data-cite-id="${internalId}">${escapeText(url)}</a>`
        : `<a href="${escapeText(url)}" target="_blank" rel="noopener noreferrer">${escapeText(url)}</a>`;
    } else if (bareId) {
      html += `<a href="#" class="jur-citation jur-citation--id" data-cite-id="${bareId}">${bareId}</a>`;
    }
    lastIndex = idx + full.length;
  }
  html += escapeText(paragraph.slice(lastIndex));
  return html;
}

/**
 * Détecte un paragraphe très probablement corrompu (caractère de
 * remplacement U+FFFD ou caractères de contrôle en forte densité) — signe
 * d'une extraction texte ratée côté Légifrance sur une portion du PDF
 * (ex. flux binaire d'une image confondu avec du texte). Affiché tel quel,
 * ce genre de contenu ne montre à l'utilisateur qu'une suite de losanges
 * illisibles, sans aucune valeur informative.
 */
function isLikelyCorrupted(paragraph: string): boolean {
  if (!paragraph) return false;
  const badChars = (paragraph.match(/[\uFFFD\x00-\x08\x0E-\x1F]/g) || []).length;
  return badChars / paragraph.length > 0.15;
}

/**
 * Met en forme un texte BRUT (sans balises HTML — ex. `Attachment.content`,
 * extraction texte d'un PDF joint) en paragraphes HTML lisibles, au lieu de
 * l'afficher tel quel en un seul bloc : le HTML ignore les retours à la
 * ligne simples d'un texte brut, ce qui rend un long texte totalement plat
 * ("affiché au kilomètre") même si le texte source contient de vrais sauts
 * de ligne. On échappe le texte nous-mêmes (il n'est pas déjà du HTML) puis
 * on découpe sur les lignes vides ; à défaut de lignes vides détectées, sur
 * les sauts de ligne simples.
 *
 * Au passage : linkifie les URLs/renvois internes (cf.
 * `linkifyPlainTextParagraph`) et tronque le texte dès qu'un paragraphe
 * semble corrompu (cf. `isLikelyCorrupted`) plutôt que d'afficher une suite
 * de caractères de remplacement illisibles jusqu'à la fin du document.
 */
export function formatPlainTextAsHtml(text: string): string {
  if (!text || !text.trim()) return '';
  const normalized = text.replace(/\r\n/g, '\n').trim();
  let paragraphs = normalized.split(/\n\s*\n+/).map((p) => p.trim()).filter(Boolean);
  // Pas de ligne vide détectée (texte extrait d'un PDF sans double saut) →
  // on découpe sur les sauts simples plutôt que de tout garder en un bloc.
  if (paragraphs.length <= 1) {
    paragraphs = normalized.split(/\n/).map((p) => p.trim()).filter(Boolean);
  }
  const out: string[] = [];
  for (const p of paragraphs) {
    if (isLikelyCorrupted(p)) {
      out.push('<p class="jur-muted">⚠️ Le reste du texte n\'a pas pu être extrait proprement par l\'API (erreur d\'encodage probable dans le document source) — consultez le fichier PDF joint pour la suite.</p>');
      break;
    }
    out.push(`<p>${linkifyPlainTextParagraph(p)}</p>`);
  }
  return out.join('');
}

// ── Sanitizer HTML minimal (sans dépendance externe) ─────────────────────────
// Conserve la sémantique Légifrance (articleNote, sommaire, renvois…) mais
// élimine tout ce qui peut exécuter du JS ou casser la mise en page.

const ALLOWED_TAGS = new Set([
  'p','br','hr','div','section','article','aside','header','footer',
  'h1','h2','h3','h4','h5','h6',
  'ul','ol','li','dl','dt','dd',
  'table','thead','tbody','tfoot','tr','th','td','caption','colgroup','col',
  'strong','b','em','i','u','s','del','ins','mark','abbr','cite','q','sub','sup',
  'span','a','blockquote','pre','code','kbd','samp','var',
  'details','summary',
  'figure','figcaption',
  'time', 'small',
]);

const ALLOWED_ATTRS: Record<string, string[]> = {
  a:       ['href', 'title', 'target', 'rel'],
  abbr:    ['title'],
  time:    ['datetime'],
  td:      ['colspan', 'rowspan'],
  th:      ['colspan', 'rowspan', 'scope'],
  col:     ['span'],
  colgroup:['span'],
  // Tous les éléments peuvent avoir class + data-* + id + style limité
  '*':     ['class', 'id', 'title', 'lang', 'dir'],
};

/** Sanitise un HTML brut et retourne du HTML sûr. */
export function sanitizeHtml(html: string): string {
  if (!html) return '';
  // Utilise le DOMParser du navigateur (disponible dans Tauri WebView)
  const doc = new DOMParser().parseFromString(html, 'text/html');

  function cleanNode(node: Node): Node | null {
    if (node.nodeType === Node.TEXT_NODE) return node.cloneNode();
    if (node.nodeType !== Node.ELEMENT_NODE) return null;
    const el = node as Element;
    const tag = el.tagName.toLowerCase();

    // Supprime balises dangereuses et leur contenu
    if (['script','style','svg','math','form','input','button','select','textarea',
         'iframe','frame','object','embed','applet','link','meta','base'].includes(tag)) {
      return null;
    }

    // Si la balise n'est pas dans la liste autorisée, on garde le contenu mais
    // on remplace la balise par un span neutre (pour conserver le texte).
    const outTag = ALLOWED_TAGS.has(tag) ? tag : 'span';
    const out = document.createElement(outTag);

    // Copie les attributs autorisés
    const globalAllowed = ALLOWED_ATTRS['*'] || [];
    const tagAllowed    = ALLOWED_ATTRS[outTag] || [];
    const allowed       = new Set([...globalAllowed, ...tagAllowed]);
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      if (allowed.has(name) || name.startsWith('data-')) {
        // Bloque les attributs dangereux même dans les listes autorisées
        const val = attr.value;
        if (/javascript:/i.test(val) || /^on/i.test(name)) continue;
        out.setAttribute(name, val);
      }
    }

    // Force les liens externes à s'ouvrir prudemment
    if (outTag === 'a') {
      const href = out.getAttribute('href') || '';
      if (href.startsWith('http')) {
        out.setAttribute('target', '_blank');
        out.setAttribute('rel', 'noopener noreferrer');
      }
    }

    // Récursion sur les enfants
    for (const child of Array.from(el.childNodes)) {
      const cleaned = cleanNode(child);
      if (cleaned) out.appendChild(cleaned);
    }
    return out;
  }

  const container = document.createElement('div');
  for (const child of Array.from(doc.body.childNodes)) {
    const cleaned = cleanNode(child);
    if (cleaned) container.appendChild(cleaned);
  }
  return container.innerHTML;
}

/** Strip toutes les balises HTML — utilisé pour les extraits de texte courts
 *  (titres de résultats, breadcrumb, tooltip). */
export function stripHtml(html: string): string {
  if (!html) return '';
  // Chemin rapide sans DOMParser pour les appels fréquents
  return html.replace(/<[^>]+>/g, '').replace(/&amp;/g,'&').replace(/&lt;/g,'<')
             .replace(/&gt;/g,'>').replace(/&nbsp;/g,' ').replace(/&quot;/g,'"')
             .replace(/&#39;/g,"'").replace(/\s+/g,' ').trim();
}

// ── Conversion HTML → Markdown (uniquement pour le prompt LLM) ───────────────
// Dans le viewer on rend du HTML directement. Turndown reste utile uniquement
// pour construire les prompts texte envoyés au LLM.

const turndownService = new TurndownService({
  headingStyle: 'atx',
  bulletListMarker: '-',
  codeBlockStyle: 'fenced',
});
turndownService.remove(['script', 'style']);

export function toHtmlString(value: unknown): string {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(toHtmlString).filter(Boolean).join('<br>');
  if (typeof value === 'object') return '';
  return String(value);
}

/** Convertit HTML en Markdown — uniquement pour les prompts LLM. */
export function htmlToMarkdown(value: unknown): string {
  const html = toHtmlString(value);
  if (!html) return '';
  try { return turndownService.turndown(html).trim(); }
  catch { return stripHtml(html); }
}

// ── Helpers d'affichage HTML ─────────────────────────────────────────────────

/** Enveloppe un bloc HTML dans un conteneur sémantique avec classe CSS. */
export function wrapHtml(tag: string, cls: string, inner: string): string {
  return inner ? `<${tag} class="${cls}">${inner}</${tag}>` : '';
}

/** Construit un paragraphe de métadonnées en HTML. */
export function metaHtml(pairs: [string, string | undefined][]): string {
  const items = pairs
    .filter(([, v]) => v)
    .map(([k, v]) => `<span class="jur-meta-key">${k}</span><span class="jur-meta-val">${stripHtml(v!)}</span>`);
  return items.length
    ? `<dl class="jur-doc-meta">${items.map(i => `<div class="jur-doc-meta-row">${i}</div>`).join('')}</dl>`
    : '';
}

// ── Citations Légifrance dans le HTML ─────────────────────────────────────────
// Légifrance produit des URLs du type :
//   /affichCodeArticle.do?cidTexte=LEGITEXT...&idArticle=LEGIARTI...
//   /affichTexte.do?cidTexte=JORFTEXT...
// On les transforme en attributs data-cite-id pour la navigation interne.

export function linkifyLegifranceUrls(html: string): string {
  // Liens internes Légifrance → data-cite-id
  return html.replace(
    /href="[^"]*(?:cidTexte|textId)=([A-Z]+\d{12})[^"]*"/g,
    (match, id) => `href="#" data-cite-id="${id}" class="jur-citation jur-citation--id"`
  ).replace(
    /href="[^"]*idArticle=([A-Z]+\d{12})[^"]*"/g,
    (match, id) => `href="#" data-cite-id="${id}" class="jur-citation jur-citation--id"`
  );
}

/** Détecte les identifiants Légifrance orphelins dans le texte brut et les
 *  transforme en liens cliquables.  N'opère que sur les TextNodes. */
export function injectCitationLinks(html: string): string {
  // Identifiants nus (hors attributs href/data-*) → <a data-cite-id>
  return html.replace(
    /(?<![="'])(\b(?:LEGIARTI|JORFTEXT|JURITEXT|KALITEXT|LEGITEXT|KALICONT|JORFCONT)\d{12}\b)/g,
    '<a href="#" class="jur-citation jur-citation--id" data-cite-id="$1">$1</a>'
  );
}

// ── Résolution inline des citations d'article (visible sans interaction) ────
// Historiquement, un lien de citation (`data-cite-id`) n'affichait le texte
// de l'article visé qu'au survol (bulle de prévisualisation, cf.
// handleLinkMouseEnter dans useDocumentViewer.ts) ou après un clic de
// navigation. Problème : tout le contenu affiché est susceptible d'être
// envoyé tel quel à un LLM pour analyse (bouton « Analyser avec l'IA »,
// export de dossier…) — un texte qui dépend d'un survol de souris qui n'a
// jamais eu lieu est invisible pour ce cas d'usage. Légifrance, de son côté,
// affiche le texte cité directement sous le lien (aperçu déroulant ouvert
// par défaut sur certains renvois d'articles).
//
// On résout donc ici le texte des articles cités et on l'insère DANS le HTML
// final, avant que `contenuLlm` (dérivé de `content.body` par simple
// stripHtml, cf. openDocument) ne soit calculé — aucun changement
// supplémentaire n'est nécessaire côté LLM, il voit le texte inline comme
// n'importe quel autre paragraphe.
//
// Choix délibérés :
//  - Un seul niveau de résolution (le texte inséré n'est pas lui-même
//    re-parcouru pour résoudre SES propres citations) : au-delà, le nombre
//    d'appels réseau croît trop vite pour un gain de lisibilité marginal —
//    le lien cliquable reste disponible dans le texte inséré pour qui veut
//    aller plus loin.
//  - Uniquement les liens vers un ARTICLE (LEGIARTI/JORFARTI/KALIARTI/
//    CNILARTICLE) : ce sont les seuls dont le texte cité est court et
//    directement pertinent inline ; un lien vers un texte entier (LEGITEXT/
//    JORFTEXT) resterait un lien simple (insérer un texte de loi complet
//    inline n'aurait pas de sens).
//  - Plafond de liens résolus par document (maxLinks) : au-delà, le lien
//    reste un simple lien cliquable (comportement inchangé) plutôt que de
//    déclencher un nombre non borné d'appels réseau au chargement. Fixé à
//    40 — pas 12 comme dans une première version : un décret modificatif
//    « ordinaire » (ex. un décret à 7 articles, chacun modifiant 1 à 3
//    articles d'un texte antérieur) cite déjà 10 à 15 identifiants
//    UNIQUES rien que dans ses premiers articles, et le compteur est
//    partagé sur tout le document (pas par article) : un plafond trop bas
//    fait taire silencieusement la résolution inline sur la deuxième
//    moitié d'un document tout à fait ordinaire, pas seulement sur les cas
//    extrêmes qu'il est censé protéger. 40 reste très en-deçà d'un
//    problème réel de charge réseau (appels parallèles + cache) tout en
//    couvrant la quasi-totalité des textes réglementaires courants.
//  - Ne réutilise pas `sanitizeHtml`/DOMParser ici : cette fonction est
//    volontairement indépendante de la fonction de fetch (fournie par
//    l'appelant), pour ne pas coupler ce module — par ailleurs pur — à
//    `fetch`/API_BASE. Le HTML retourné par `fetchArticleHtml` doit déjà
//    être sanitisé par l'appelant (cf. extractArticleContent, qui sanitise
//    systématiquement le corps d'article).

const ARTICLE_ID_RE = /^(LEGIARTI|JORFARTI|KALIARTI|CNILARTICLE)\d{12}$/;

/** Plafond par défaut de `resolveInlineCitations` (cf. commentaire ci-dessus
 *  sur son choix). Exporté pour permettre à l'appelant de comparer le nombre
 *  total de citations d'un document à ce plafond (bouton « Charger tous les
 *  articles », affiché uniquement quand le plafond a effectivement tronqué
 *  la résolution). */
export const DEFAULT_MAX_INLINE_CITATIONS = 40;

/** Dénombre les identifiants d'ARTICLE (LEGIARTI/JORFARTI/KALIARTI/
 *  CNILARTICLE) uniques cités par lien (`data-cite-id`) dans un document,
 *  sans plafond — contrairement à `resolveInlineCitations`, qui s'arrête à
 *  `maxLinks`. Sert uniquement à savoir si CE document a plus de citations
 *  que ce que `resolveInlineCitations` résout par défaut (pour afficher ou
 *  non un bouton « Charger tous les articles »), sans déclencher le moindre
 *  appel réseau. */
export function countCitableArticleIds(html: string): number {
  if (!html || !html.includes('data-cite-id')) return 0;
  const seen = new Set<string>();
  const re = /data-cite-id="([^"]+)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html)) !== null) {
    const id = m[1];
    if (ARTICLE_ID_RE.test(id)) seen.add(id);
  }
  return seen.size;
}

export async function resolveInlineCitations(
  html: string,
  fetchArticleHtml: (id: string) => Promise<string | null>,
  maxLinks = DEFAULT_MAX_INLINE_CITATIONS,
): Promise<string> {
  if (!html || !html.includes('data-cite-id')) return html;

  // Collecte des ids d'articles cités, dans l'ordre d'apparition, dédupliqués
  // (un même article peut être cité plusieurs fois dans un texte).
  const seen = new Set<string>();
  const ids: string[] = [];
  const re = /data-cite-id="([^"]+)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html)) !== null) {
    const id = m[1];
    if (ARTICLE_ID_RE.test(id) && !seen.has(id)) {
      seen.add(id);
      ids.push(id);
      if (ids.length >= maxLinks) break;
    }
  }
  if (!ids.length) return html;

  // Récupération par lots de taille limitée (CONCURRENCY) plutôt qu'un
  // unique Promise.all sur la totalité des ids : à 40 liens plafond, un
  // Promise.all direct enverrait jusqu'à 40 requêtes strictement
  // simultanées vers l'API PISTE (Légifrance) au chargement d'un seul
  // document, sans qu'aucun mécanisme de rate-limiting n'existe par
  // ailleurs dans le projet (ni ici, ni côté backend Rust) pour l'absorber.
  // Le lotissement garde l'essentiel du gain de latence du parallélisme
  // (6 requêtes de front plutôt qu'1 seule à la fois) sans reproduire une
  // rafale de 40 requêtes d'un coup. Tolérant aux échecs individuels (un
  // article dé-référencé ou une erreur réseau ponctuelle ne doit pas faire
  // échouer l'affichage du document entier, cf. `.catch(() => null)`).
  const CONCURRENCY = 6;
  const entries: Array<readonly [string, string | null]> = [];
  for (let i = 0; i < ids.length; i += CONCURRENCY) {
    const batch = ids.slice(i, i + CONCURRENCY);
    const batchResults = await Promise.all(
      batch.map(async (id) => {
        try { return [id, await fetchArticleHtml(id)] as const; }
        catch { return [id, null] as const; }
      }),
    );
    entries.push(...batchResults);
  }
  const resolved = new Map(entries.filter(([, txt]) => !!txt) as Array<[string, string]>);
  if (!resolved.size) return html;

  // Insertion du bloc de texte cité juste après le lien correspondant. On
  // n'insère qu'à la PREMIÈRE occurrence de chaque id dans le document (les
  // occurrences suivantes restent de simples liens cliquables) pour éviter
  // de répéter un même texte plusieurs fois si l'article est cité à
  // plusieurs endroits.
  const inserted = new Set<string>();
  return html.replace(
    /(<a\b[^>]*\bdata-cite-id="([^"]+)"[^>]*>.*?<\/a>)/g,
    (fullMatch, anchorHtml: string, id: string) => {
      const txt = resolved.get(id);
      if (!txt || inserted.has(id)) return fullMatch;
      inserted.add(id);
      return `${anchorHtml}<div class="jur-cited-article">${txt}</div>`;
    },
  );
}

// ── Renderers de sections / articles ─────────────────────────────────────────

// Rendu du bandeau de versions d'un article (ConsultArticle.multipleVersions /
// .articleVersions). NB : ces métadonnées sont documentées sur le schéma
// `Article`/`ConsultArticle` en général, mais rien ne garantit qu'un endpoint
// « allégé » comme /consult/legiPart (qui renvoie une PHOTO du texte à une
// date donnée, avec ses articles imbriqués) les peuple réellement — seul
// /consult/getArticle (fiche dédiée à un seul article) est un candidat sûr.
// Si `articleVersions` est absent/vide ici mais que l'article a un id, on
// propose quand même un lien vers la fiche article dédiée : c'est là que la
// liste de versions, si elle existe côté API, a le plus de chances d'être
// renvoyée.
//
// IMPORTANT : chaque lien de version porte aussi `data-cite-cid` (le
// Chronical ID de l'article, stable à travers ses versions). `/consult/
// getArticle` avec l'id d'UNE version historique précise ne renvoie pas
// fiablement le contenu de cette version (il semble toujours résoudre vers
// la version actuellement en vigueur) : le survol/clic sur ces liens passe
// donc par l'endpoint dédié `/article-version/:cid/:versionId`, qui va
// chercher la vraie liste des versions via getArticleByCid et sélectionne
// la bonne — cf. handleLinkMouseEnter / handleCitationClick dans
// JuridiquePanel.tsx.
export function renderArticleVersionsBlock(a: Record<string, unknown>, currentId: string, standalone = false): string {
  const versions = Array.isArray(a.articleVersions) ? a.articleVersions as Array<Record<string, unknown>> : [];
  const cid = stripHtml(toHtmlString(a.cid));
  if (versions.length > 1) {
    return `<div class="jur-article-versions">
        <span class="jur-article-versions-label">Versions de cet article :</span>
        ${versions.map((v) => {
          const vId = stripHtml(toHtmlString(v.id));
          const vDebut = formatLegiDate(v.dateDebut);
          const vFin = formatLegiDate(v.dateFin);
          const vEtat = stripHtml(toHtmlString(v.etat));
          const isCurrent = vId === currentId;
          const label = [vDebut ? `à partir du ${vDebut}` : '', vFin ? `jusqu'au ${vFin}` : '', vEtat || ''].filter(Boolean).join(' — ');
          const cidAttr = cid ? ` data-cite-cid="${cid}"` : '';
          return isCurrent
            ? `<span class="jur-article-version jur-article-version--current">${label || 'Version actuelle'}</span>`
            : `<a href="#" class="jur-article-version" data-cite-id="${vId}"${cidAttr}>${label || vId}</a>`;
        }).join('')}
      </div>`;
  }
  if (standalone) {
    // Vue article seul (/consult/getArticle) : c'est l'endroit où la liste
    // de versions, si elle existe, est censée être renvoyée en entier. Si
    // elle est absente ici, on le dit explicitement plutôt que de rester
    // silencieux (utile pour distinguer « une seule version existe » de
    // « l'API n'a pas renvoyé cette information »).
    return `<p class="jur-muted">Aucun historique de versions renvoyé par l'API pour cet article.</p>`;
  }
  // Dans la liste imbriquée d'un texte (/consult/legiPart), rien ne garantit
  // que `articleVersions`/`multipleVersions` soient peuplés (endpoint plus
  // léger) : on propose systématiquement d'ouvrir la fiche dédiée de
  // l'article, seul endroit où l'historique complet est garanti par le
  // schéma officiel, plutôt que de se fier à un drapeau qui peut être absent
  // même quand des versions existent réellement.
  if (currentId) {
    return `<div class="jur-article-versions">
        <a href="#" class="jur-article-version jur-article-version--link" data-cite-id="${currentId}">↻ Voir l'historique des versions de cet article</a>
      </div>`;
  }
  return '';
}

export function renderArticlesList(articles: unknown[]): string {
  return (articles as Array<Record<string, unknown>>).map((a) => {
    const num  = stripHtml(toHtmlString(a.num));
    const tit  = stripHtml(toHtmlString(a.titre || a.title));
    const html = toHtmlString(a.content || a.texteHtml || a.texte || a.contenu);
    const nota = toHtmlString(a.notaHtml || a.nota);
    const etat = stripHtml(toHtmlString(a.etat));
    const id   = stripHtml(toHtmlString(a.id));
    // Versions de l'article — voir renderArticleVersionsBlock ci-dessus pour
    // le détail des deux cas gérés (liste inline vs repli vers la fiche
    // dédiée). On ne reproduit pas la comparaison côte à côte de Légifrance
    // (ça demanderait un panneau dédié à deux volets), mais on donne accès
    // aux autres versions via le mécanisme de navigation par citation
    // (data-cite-id) déjà utilisé pour les renvois d'articles.
    const versionsHtml = renderArticleVersionsBlock(a, id);
    // A2 — schéma ConsultArticle (article imbriqué dans un texte, via
    // /consult/legiPart et consorts) : le nom de champ est ici
    // `lstLienModification` (≠ `lienModifications` sur /consult/getArticle,
    // cf. note détaillée dans renderArticleModificationsSection). On garde
    // volontairement cette section compacte dans la liste imbriquée (pas de
    // bloc « Historique » ici, qui alourdirait l'affichage d'un texte
    // entier) — l'historique complet reste consultable via la fiche
    // article dédiée (lien de repli déjà proposé par renderArticleVersionsBlock).
    const modificationsHtml = renderArticleModificationsSection(a.lstLienModification ?? a.lienModifications);
    // data-num alimente le numéro affiché en marge gauche (::before CSS) —
    // on garde aussi le numéro dans le <h3> pour la lisibilité hors marge
    // (petits écrans, impression, lecteurs d'écran).
    return `<article class="jur-article" id="${id}" data-num="${num ? num.replace(/"/g, '&quot;') : ''}">
      <h3 class="jur-article-num">Article ${num}${tit ? ` — ${tit}` : ''}${etat && etat !== 'VIGUEUR' ? ` <span class="jur-tag-inline jur-tag-inline--muted">${etat}</span>` : ''}</h3>
      ${versionsHtml}
      ${html ? sanitizeHtml(linkifyLegifranceUrls(injectCitationLinks(html))) : ''}
      ${nota ? `<aside class="jur-nota"><strong>Nota</strong><br>${sanitizeHtml(nota)}</aside>` : ''}
      ${modificationsHtml}
    </article>`;
  }).join('\n');
}

// `pathPrefix` désambiguïse les sections dépourvues d'id/cid exploitable
// (rare, mais déjà vu sur certaines conventions collectives KALI) : chaque
// niveau de récursion ajoute son index dans la fratrie, ce qui garantit un
// id unique et stable même sans identifiant Légifrance — nécessaire pour
// que le plan interactif (cf. useDocumentViewer.buildTocTree) puisse
// distinguer chaque nœud et lui associer un état plié/déplié propre.
export function renderSections(sections: unknown[], depth = 0, pathPrefix = ''): string {
  return (sections as Array<Record<string, unknown>>).map((s, idx) => {
    const tit  = stripHtml(toHtmlString(s.titre || s.title || s.intitule));
    const arts = (s.articles ?? s.listArticle ?? []) as unknown[];
    const sub  = (s.sections ?? s.listSection ?? s.children ?? []) as unknown[];
    const hTag = depth === 0 ? 'h2' : depth === 1 ? 'h3' : 'h4';
    const path = `${pathPrefix}${idx}`;
    // Id stable de la section : priorité à l'identifiant Légifrance réel
    // (id/cid — LEGISCTA...) pour rester cohérent avec les liens de citation
    // existants (data-cite-id) qui, eux, référencent déjà ces ids ; repli sur
    // un id positionnel (chemin dans l'arbre) si absent. Préfixé pour ne
    // jamais entrer en collision avec un id d'article (cf. renderArticlesList,
    // qui utilise l'id brut sans préfixe).
    const realId = stripHtml(toHtmlString(s.id || s.cid)).replace(/[^a-zA-Z0-9_-]/g, '');
    const sectionId = `jur-toc-sct-${realId || `p${path}`}`;
    return `<section class="jur-section jur-section--depth-${depth}" id="${sectionId}">
      ${tit ? `<${hTag} class="jur-section-title">${tit}</${hTag}>` : ''}
      ${arts.length ? renderArticlesList(arts) : ''}
      ${sub.length ? renderSections(sub, depth + 1, `${path}-`) : ''}
    </section>`;
  }).join('\n');
}

// ── Historique et modifications d'un article (A2) ────────────────────────────
// Deux champs du schéma Légifrance `Article` (réponse réelle de /consult/
// getArticle, cf. GetArticleResponse.article), jamais rendus jusqu'ici alors
// qu'ils sont déjà présents dans le payload récupéré par le frontend :
//
//  - `historique` : texte HTML de l'historique légal de l'article.
//  - `lienModifications` : liste des textes qui ont modifié cet article
//    (schéma LienModification). ⚠️ Ne pas confondre avec `lstLienModification`
//    (nom de champ du schéma voisin `ConsultArticle`, utilisé uniquement
//    quand l'article est imbriqué dans un texte via /consult/legiPart et
//    consorts — cf. ConsultTextResponse.articles). Les deux noms sont donc
//    acceptés ici pour que ce même renderer serve aussi bien /consult/
//    getArticle (article seul, standalone) que la liste d'articles d'un texte
//    (renderArticlesList) sans dupliquer la logique.
//
// `LienModification.articleId` est déjà un identifiant LEGIARTI exploitable
// tel quel par le mécanisme `data-cite-id` existant — contrairement au `visa`
// Judilibre (cf. extract-judilibre.ts/renderVisaSection), aucune extraction
// depuis une URL n'est nécessaire ici : c'est un cas plus simple.
// `textCid`, lui, pointe vers le TEXTE entier modificateur (LOI, décret…),
// pas vers l'article — utile en repli quand `articleId` est absent.

/** Rendu de la section « Historique » d'un article (texte HTML `historique`,
 *  champ `Article.historique`/`ConsultArticle.historique`). */
export function renderArticleHistoriqueSection(historique: unknown): string {
  // Comme le corps de l'article lui-même (cf. extractArticleContent), le
  // texte de l'historique peut contenir des liens internes Légifrance
  // (renvois vers d'autres textes modificateurs) : on les fait passer par
  // le même pipeline linkify → citation → sanitize pour bénéficier de la
  // navigation interne (data-cite-id) plutôt que de liens morts ou externes.
  const html = sanitizeHtml(linkifyLegifranceUrls(injectCitationLinks(toHtmlString(historique))));
  if (!stripHtml(html)) return '';
  return `<section class="jur-section jur-section--related jur-section--historique">
      <h2>Historique</h2>
      <div class="jur-historique-body">${html}</div>
    </section>`;
}

/** Rendu de la section « Modifié par » d'un article (liste `LienModification`
 *  — champ `Article.lienModifications` ou `ConsultArticle.lstLienModification`
 *  selon l'endpoint, cf. note ci-dessus). */
export function renderArticleModificationsSection(liens: unknown): string {
  const arr = Array.isArray(liens) ? liens.filter((l) => l && typeof l === 'object') : [];
  if (!arr.length) return '';
  const items = arr.map((l) => {
    const lien = l as Record<string, unknown>;
    const articleId = stripHtml(toHtmlString(lien.articleId));
    const textCid = stripHtml(toHtmlString(lien.textCid));
    const textTitle = stripHtml(toHtmlString(lien.textTitle)) || textCid || 'Texte modificateur';
    const articleNum = stripHtml(toHtmlString(lien.articleNum));
    const linkType = stripHtml(toHtmlString(lien.linkType));
    const dateSig = formatLegiDate(lien.dateSignaTexte);
    const datePubli = formatLegiDate(lien.datePubliTexte);
    // On préfère un lien direct vers l'article modificateur précis
    // (articleId, LEGIARTI) quand il est fourni ; à défaut, repli sur le
    // texte modificateur entier (textCid, LOI/décret complet).
    const head = articleId
      ? `<a href="#" class="jur-citation jur-citation--id jur-related-link" data-cite-id="${articleId}">${textTitle}${articleNum ? ` — art. ${articleNum}` : ''}</a>`
      : textCid
        ? `<a href="#" class="jur-citation jur-citation--id jur-related-link" data-cite-id="${textCid}">${textTitle}</a>`
        : `<span class="jur-related-link jur-related-link--plain">${textTitle}</span>`;
    const metaLine = [linkType, dateSig ? `signé le ${dateSig}` : '', datePubli ? `publié le ${datePubli}` : '']
      .filter(Boolean).join(' · ');
    return `<div class="jur-related-item">
        <div class="jur-related-item-head">${head}</div>
        ${metaLine ? `<div class="jur-related-item-meta">${metaLine}</div>` : ''}
      </div>`;
  }).join('');
  return `<section class="jur-section jur-section--related jur-section--modifications">
      <h2>Modifié par</h2>
      <div class="jur-related-list">${items}</div>
    </section>`;
}

/** Référence lisible d'une décision Judilibre pour affichage (badge d'en-tête,
 *  export…). L'id interne Judilibre est un hash Mongo opaque (24 caractères
 *  hexadécimaux) sans signification pour un juriste — contrairement aux id
 *  Légifrance (NOR, JORFTEXT…) qui sont eux-mêmes des références officielles
 *  lisibles. On préfère donc afficher l'ECLI, sinon le(s) n° de pourvoi,
 *  et seulement en dernier recours l'id brut si aucune référence n'est
 *  disponible. */
export function judilibreDisplayRef(raw: unknown, fallbackId: string): string {
  const obj = (raw && typeof raw === 'object') ? raw as Record<string, unknown> : {};
  const ecli = typeof obj.ecli === 'string' ? obj.ecli.trim() : '';
  if (ecli) return ecli;
  const numbers = Array.isArray(obj.numbers) ? (obj.numbers as unknown[]).filter((n) => typeof n === 'string' && n.trim()) as string[] : [];
  if (numbers.length) return `Pourvoi n° ${numbers.join(', ')}`;
  return fallbackId;
}
