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
// Extraction décision JUDILIBRE
// ============================================================================
// Schéma officiel : decisionFull (GET /decision) — cf. swagger JUDILIBRE.
// Extrait de JuridiquePanel.tsx (étape 2 du refactor).
// Licence : GNU Affero General Public License v3.0 (AGPL-3.0)
// ============================================================================

import { stripHtml, toHtmlString, sanitizeHtml, metaHtml, injectCitationLinks, resolveLegifranceInternalId } from './html-helpers';
import type { ViewedContent } from './types';

/** Repère un numéro de pourvoi français (ex. « 09-12.404 », « 17-11.442 »)
 *  dans le `title` d'un rapprochement sans id/url. */
function extractPourvoiNumber(title: string): string | null {
  const m = title.match(/\b(\d{2}[-.]\d{2}\.\d{3})\b/);
  return m ? m[1] : null;
}

/** Devine le fond Légifrance le plus pertinent pour rechercher un texte visé
 *  (`visa[].title`, ex. « Article L. 242-1 du code de la sécurité sociale »,
 *  « Loi n° 85-677 du 5 juillet 1985 ») — heuristique best-effort, pas une
 *  résolution garantie : CODE_ETAT si le titre mentionne un code, LODA_ETAT
 *  sinon (lois, décrets, ordonnances). */
function guessVisaFond(title: string): 'CODE_ETAT' | 'LODA_ETAT' {
  return /\bcode\b/i.test(title) ? 'CODE_ETAT' : 'LODA_ETAT';
}

/**
 * Retire le préfixe « Article <numéro> du/de la/de l'/des » d'un `visa.title`
 * pour ne garder que la référence du TEXTE (ex. « décret n° 2008-281 du 21
 * mars 2008 », « code de la sécurité sociale ») — la recherche plein texte
 * Légifrance échoue en pratique sur la combinaison « article <n> du <texte> »
 * (confirmé par test manuel : « article 5 du décret n° 2008-281 du 21 mars
 * 2008 » → 0 résultat, alors que « décret n° 2008-281 du 21 mars 2008 » seul
 * fonctionne). On perd la précision de l'article visé, mais on gagne un
 * résultat de recherche exploitable plutôt qu'une page vide ; à charge pour
 * l'utilisateur de retrouver l'article précis dans la table des matières du
 * texte une fois celui-ci ouvert.
 */
function stripArticlePrefix(title: string): string {
  const m = title.match(/^Article\s+.+?\s+(?:du|de la|de l['’]|des)\s+(.+)$/i);
  return m ? m[1] : title;
}

/** Formate une date ISO courte (`decisionLink.date`, `fileLink.date`) en
 *  français long, silencieusement si absente/invalide. */
function formatShortDate(raw: unknown): string {
  const s = stripHtml(toHtmlString(raw));
  if (!s) return '';
  const d = new Date(s);
  return isNaN(d.getTime()) ? '' : d.toLocaleDateString('fr-FR', { day: '2-digit', month: 'long', year: 'numeric' });
}

/**
 * Rendu d'un objet `decisionLink` (contested/forward/rapprochements/timeline)
 * sous forme de carte compacte.
 *
 * Navigation : on ne rend le lien cliquable (`data-cite-id`) QUE si `id`
 * ressemble à un identifiant JUDILIBRE (hash Mongo, 24 caractères hex) — le
 * même motif que `resolvePreviewEndpoint`/`openDocument` utilisent pour
 * détecter une décision JUDILIBRE indépendamment du fond/source courant du
 * formulaire de recherche. Un `id` absent ou de forme différente n'est pas
 * fiable pour la navigation interne : on retombe alors sur `url` (lien
 * externe), puis sur du texte simple.
 *
 * Cas particuliers documentés par le swagger : `ongoing` (pourvoi non encore
 * jugé : décision cible inexistante), `partial` (décision publiée par
 * extraits) et `content` (texte de repli quand la décision référencée n'est
 * pas indexée dans JUDILIBRE).
 */
function renderDecisionLinkCard(link: unknown): string {
  if (!link || typeof link !== 'object') return '';
  const l = link as Record<string, unknown>;
  const title = stripHtml(toHtmlString(l.title)) || 'Décision';
  const number = stripHtml(toHtmlString(l.number));
  const jurisdiction = stripHtml(toHtmlString(l.jurisdiction));
  const location = stripHtml(toHtmlString(l.location));
  const chamber = stripHtml(toHtmlString(l.chamber));
  const solution = stripHtml(toHtmlString(l.solution));
  const dateFmt = formatShortDate(l.date);
  const description = stripHtml(toHtmlString(l.description));
  const themes = Array.isArray(l.theme) ? (l.theme as unknown[]).map((t) => stripHtml(toHtmlString(t))).filter(Boolean) : [];
  const id = typeof l.id === 'string' ? l.id.trim() : '';
  const url = typeof l.url === 'string' ? l.url.trim() : '';
  const ongoing = l.ongoing === true;
  const partial = l.partial === true;
  const content = typeof l.content === 'string' ? stripHtml(l.content) : '';

  const metaLine = [jurisdiction, location && location !== jurisdiction ? location : '', chamber && chamber !== jurisdiction ? chamber : '', number ? `n° ${number}` : '', dateFmt, solution]
    .filter(Boolean).join(' · ');

  const badges = [
    ongoing ? '<span class="jur-tag-inline jur-tag-inline--muted">pourvoi en cours</span>' : '',
    partial ? '<span class="jur-tag-inline jur-tag-inline--muted">extrait</span>' : '',
  ].filter(Boolean).join(' ');

  const isJudilibreId = /^[0-9a-f]{24}$/i.test(id);
  let head: string;
  if (isJudilibreId) {
    head = `<a href="#" class="jur-citation jur-citation--id jur-related-link" data-cite-id="${id}">${title}</a>`;
  } else if (url) {
    head = `<a href="${url.replace(/"/g, '&quot;')}" target="_blank" rel="noopener noreferrer" class="jur-related-link">${title}</a>`;
  } else {
    // Ni id ni url résolus par l'API (cas fréquent des rapprochements issus
    // de la source `jurinet`, cf. commentaire d'`extractPourvoiNumber`) :
    // on tente d'en extraire le n° de pourvoi pour proposer une recherche
    // JUDILIBRE interne plutôt que de rester en texte inerte.
    const pourvoi = extractPourvoiNumber(title);
    head = pourvoi
      ? `<a href="#" class="jur-related-link jur-search-link" data-search-query="${pourvoi}" data-search-source="judilibre" title="Rechercher le pourvoi n° ${pourvoi} dans l'application">${title}</a>`
      : `<span class="jur-related-link jur-related-link--plain">${title}</span>`;
  }

  const desc = description || (content ? content.slice(0, 240) + (content.length > 240 ? '…' : '') : '');

  return `<div class="jur-related-item">
      <div class="jur-related-item-head">${head}${badges ? ` ${badges}` : ''}</div>
      ${metaLine ? `<div class="jur-related-item-meta">${metaLine}</div>` : ''}
      ${desc ? `<div class="jur-related-item-desc">${desc}</div>` : ''}
      ${themes.length ? `<div class="jur-related-item-themes">${themes.join(' · ')}</div>` : ''}
    </div>`;
}

/** Section « voir aussi » listant plusieurs `decisionLink` (rapprochements,
 *  timeline). Ne s'affiche que si la liste contient au moins un élément
 *  exploitable. */
function renderDecisionLinksSection(title: string, links: unknown, extraClass = ''): string {
  const arr = Array.isArray(links) ? links.filter((l) => l && typeof l === 'object') : [];
  if (!arr.length) return '';
  return `<section class="jur-section jur-section--related${extraClass ? ` ${extraClass}` : ''}">
      <h2>${title}</h2>
      <div class="jur-related-list">${arr.map(renderDecisionLinkCard).join('')}</div>
    </section>`;
}

/**
 * Bloc « Procédure » : chaîne contested → (cette décision) → forward, puis
 * chronologie complète de l'affaire (`timeline`). Placé en tête de document
 * (juste après le bandeau de métadonnées) car c'est un contexte de lecture,
 * pas une annexe — à l'inverse des rapprochements/textes appliqués/documents
 * associés, plus utiles en fin de document façon « voir aussi ».
 */
function renderProcedureSection(contested: unknown, forward: unknown, timeline: unknown): string {
  const hasContested = contested && typeof contested === 'object' && Object.keys(contested as object).length > 0;
  const hasForward = forward && typeof forward === 'object' && Object.keys(forward as object).length > 0;
  const timelineArr = Array.isArray(timeline) ? timeline.filter((t) => t && typeof t === 'object') : [];

  if (!hasContested && !hasForward && !timelineArr.length) return '';

  const chain = (hasContested || hasForward)
    ? `<div class="jur-procedure-chain">
        ${hasContested ? `<div class="jur-procedure-step jur-procedure-step--contested">
            <span class="jur-procedure-label">Décision attaquée</span>
            ${renderDecisionLinkCard(contested)}
          </div><div class="jur-procedure-arrow" aria-hidden="true">↓</div>` : ''}
        <div class="jur-procedure-step jur-procedure-step--current"><span class="jur-procedure-label jur-procedure-label--current">Cette décision</span></div>
        ${hasForward ? `<div class="jur-procedure-arrow" aria-hidden="true">↓</div><div class="jur-procedure-step jur-procedure-step--forward">
            <span class="jur-procedure-label">Décision suivante</span>
            ${renderDecisionLinkCard(forward)}
          </div>` : ''}
      </div>`
    : '';

  const timelineHtml = timelineArr.length
    ? `<div class="jur-procedure-timeline">
        <h3 class="jur-procedure-timeline-title">Chronologie de l'affaire</h3>
        <div class="jur-related-list">${timelineArr.map(renderDecisionLinkCard).join('')}</div>
      </div>`
    : '';

  return `<section class="jur-section jur-section--procedure"><h2>Procédure</h2>${chain}${timelineHtml}</section>`;
}

/** Section « Textes appliqués » (`visa`, tableau de `textLink`). `textLink.id`
 *  est un entier interne JUDILIBRE sans route de résolution documentée — en
 *  revanche `textLink.url` pointe vers une URL publique Légifrance
 *  (`affichCodeArticle.do?idArticle=…`, `affichTexte.do?cidTexte=…`) dont on
 *  peut extraire l'identifiant LEGIARTI/LEGITEXT pour ouvrir l'article DANS
 *  l'application (même mécanisme `data-cite-id` que partout ailleurs) plutôt
 *  que de renvoyer l'utilisateur vers le site public. Repli sur un lien
 *  externe uniquement si l'URL ne porte aucun identifiant reconnaissable. */
function renderVisaSection(visa: unknown): string {
  const arr = Array.isArray(visa) ? visa.filter((t) => t && typeof t === 'object') : [];
  if (!arr.length) return '';
  const items = arr.map((t) => {
    const v = t as Record<string, unknown>;
    const title = stripHtml(toHtmlString(v.title)) || 'Texte';
    const url = typeof v.url === 'string' ? v.url.trim() : '';
    const internalId = url ? resolveLegifranceInternalId(url) : null;
    if (internalId) {
      return `<li><a href="#" class="jur-citation jur-citation--id" data-cite-id="${internalId}">${title}</a></li>`;
    }
    if (url) {
      return `<li><a href="${url.replace(/"/g, '&quot;')}" target="_blank" rel="noopener noreferrer">${title}</a></li>`;
    }
    // Aucune url du tout (cas fréquent sur les décisions `jurinet`, cf.
    // commentaire d'`extractPourvoiNumber` plus haut) : recherche Légifrance
    // interne à partir du texte du visa, à défaut d'identifiant résolu.
    const cleanTitle = stripArticlePrefix(title).replace(/\.$/, '').trim();
    const guessedFond = guessVisaFond(title);
    return `<li><a href="#" class="jur-related-link jur-search-link" data-search-query="${cleanTitle.replace(/"/g, '&quot;')}" data-search-source="legifrance" data-search-fond="${guessedFond}" title="Rechercher ce texte dans l'application">${title}</a></li>`;
  }).join('');
  return `<section class="jur-section jur-section--related"><h2>Textes appliqués</h2><ul class="jur-related-textlist">${items}</ul></section>`;
}

// Intitulés de repli pour `fileLink.type` — uniquement utilisés si l'appel
// n'a pas transmis `resolve_references=true` (auquel cas l'API renvoie déjà
// l'intitulé complet dans ce même champ). Liste non exhaustive : se référer à
// `GET /taxonomy?id=filetype` pour la liste complète et à jour.
const FILE_TYPE_FALLBACK: Record<string, string> = {
  comm_comm: 'Communiqué',
  comm_note: 'Note explicative',
  comm_trad: 'Arrêt traduit',
  prep_rapp: 'Rapport du rapporteur',
  prep_avis: "Avis de l'avocat général",
};

/** Section « Documents associés » (`files`, tableau de `fileLink`) :
 *  communiqués, notes explicatives, traductions, rapports, avis… On
 *  privilégie `rawUrl` (accessible publiquement d'après le swagger) sur
 *  `url` quand les deux sont présents. */
function renderFilesSection(files: unknown): string {
  const arr = Array.isArray(files) ? files.filter((f) => f && typeof f === 'object') : [];
  if (!arr.length) return '';
  const items = arr.map((f) => {
    const file = f as Record<string, unknown>;
    const name = stripHtml(toHtmlString(file.name)) || 'Document';
    const rawUrl = typeof file.rawUrl === 'string' ? file.rawUrl.trim() : '';
    const url = rawUrl || (typeof file.url === 'string' ? file.url.trim() : '');
    const typeRaw = stripHtml(toHtmlString(file.type));
    const typeLabel = FILE_TYPE_FALLBACK[typeRaw] || typeRaw;
    const date = formatShortDate(file.date);
    const size = stripHtml(toHtmlString(file.size));
    const meta = [typeLabel, date, size].filter(Boolean).join(' · ');
    return `<li>${url ? `<a href="${url.replace(/"/g, '&quot;')}" target="_blank" rel="noopener noreferrer">${name}</a>` : name}${meta ? `<span class="jur-related-item-meta"> — ${meta}</span>` : ''}</li>`;
  }).join('');
  return `<section class="jur-section jur-section--related"><h2>Documents associés</h2><ul class="jur-related-textlist jur-related-files">${items}</ul></section>`;
}

/** Décision JUDILIBRE — schéma officiel `decisionFull`. */
export function extractDecisionContent(obj: Record<string, unknown>): ViewedContent {
  const jurisdiction = stripHtml(toHtmlString(obj.jurisdiction));
  const chamber      = stripHtml(toHtmlString(obj.chamber));
  const formation    = stripHtml(toHtmlString(obj.formation));
  const solution     = stripHtml(toHtmlString(obj.solution));
  const ecli         = stripHtml(toHtmlString(obj.ecli));
  // Schéma officiel JUDILIBRE (`decisionFull`) : le champ est `decision_date`
  // (ou `decision_datetime`) — il n'existe ni `date` ni `dateDecision`, ce
  // qui faisait disparaître silencieusement la date de la décision.
  const dateDec      = stripHtml(toHtmlString(obj.decision_date || obj.decision_datetime));
  const dateStr      = dateDec
    ? new Date(dateDec).toLocaleDateString('fr-FR', { day: '2-digit', month: 'long', year: 'numeric' })
    : '';
  const numbers: string[] = Array.isArray(obj.numbers) ? obj.numbers as string[] : [];
  const courtLine = [jurisdiction, chamber !== jurisdiction ? chamber : '', formation !== chamber ? formation : '']
    .filter(Boolean).join(', ');
  const decisionTitle = [courtLine || 'Décision', dateStr].filter(Boolean).join(' — ');
  // `titlesAndSummaries` (uniquement pour les décisions de la Cour de
  // cassation) regroupe `titles` et `secondaryTitles` — il n'existe pas de
  // champ `titrage` au niveau racine.
  const tas = (obj.titlesAndSummaries && typeof obj.titlesAndSummaries === 'object')
    ? obj.titlesAndSummaries as Record<string, unknown> : {};
  const titrageEntries = ([] as unknown[]).concat(
    Array.isArray(tas.titles) ? tas.titles as unknown[] : [],
    Array.isArray(tas.secondaryTitles) ? tas.secondaryTitles as unknown[] : [],
  );
  const titrages = titrageEntries
    .map((t) => typeof t === 'string' ? t : stripHtml(toHtmlString((t as Record<string, unknown>)?.title ?? (t as Record<string, unknown>)?.titre)))
    .filter((t): t is string => Boolean(t && t.trim()));
  const titrageHtml = titrages.length
    ? `<section class="jur-section"><h2>Titrage</h2><ul>${titrages.map((t) => `<li>${t}</li>`).join('')}</ul></section>` : '';
  const themes: string[] = Array.isArray(obj.themes) ? obj.themes as string[] : [];
  const themesHtml = themes.length
    ? `<p class="jur-themes"><strong>Thèmes :</strong> ${themes.map(stripHtml).join(' · ')}</p>` : '';
  // `summary` (texte brut) est le seul champ de sommaire documenté par
  // l'API — les anciens noms `sommaire`/`resumePrincipal` n'existent pas.
  const sommaire  = toHtmlString(obj.summary);
  const rawText   = toHtmlString(obj.text_highlight || obj.text);
  const pub: string[] = Array.isArray(obj.publication) ? obj.publication as string[] : [];
  const meta = metaHtml([
    numbers.length ? ['N° de pourvoi', numbers.join(', ')] : ['', undefined],
    ['ECLI', ecli || undefined],
    ['Solution', solution || undefined],
    pub.length ? ['Publication', pub.join(', ')] : ['', undefined],
    ['Date', dateStr || undefined],
  ]);
  const sommaireHtml = sommaire
    ? `<section class="jur-section"><h2>Sommaire</h2><p>${sanitizeHtml(sommaire)}</p></section>` : '';

  // ── Graphe de citations (contexte en tête, jurisprudence liée en annexe) ──
  const procedureHtml = renderProcedureSection(obj.contested, obj.forward, obj.timeline);
  const rapprochementsHtml = renderDecisionLinksSection('Rapprochements de jurisprudence', obj.rapprochements);
  const visaHtml = renderVisaSection(obj.visa);
  const filesHtml = renderFilesSection(obj.files);

  // ── Découpage du texte brut en zones (JUDILIBRE API) ──────────────────
  // L'API retourne zones: { introduction, expose, moyens, motivations, dispositif, annexes }
  // chacune étant un tableau de { start, end } sur le texte brut.
  const zones = obj.zones as Record<string, Array<{start: number; end: number}>> | undefined;

  /** Convertit un segment de texte brut (ou text_highlight avec <em>) en HTML avec paragraphes.
   *  Si le segment contient déjà des balises <em> (text_highlight), on les préserve. */
  function textSegmentToHtml(segment: string): string {
    const hasMarkup = /<em>/.test(segment);
    return segment
      .split(/\n+/)
      .map(p => p.trim())
      .filter(Boolean)
      .map(p => {
        // Texte brut sans balisage : échapper HTML puis injecter liens
        if (!hasMarkup) {
          return `<p>${injectCitationLinks(p.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'))}</p>`;
        }
        // text_highlight : contient <em>…</em> — on préserve, juste injection de liens
        return `<p>${injectCitationLinks(p)}</p>`;
      })
      .join('');
  }

  const ZONE_LABELS: Record<string, string> = {
    introduction:  'Introduction',
    expose:        'Exposé du litige',
    moyens:        'Moyens',
    motivations:   'Motivations',
    dispositif:    'Dispositif',
    annexes:       'Moyens annexés',
  };
  const ZONE_ORDER = ['introduction', 'expose', 'moyens', 'motivations', 'dispositif', 'annexes'];

  let texteHtml = '';
  if (rawText) {
    // Si l'API a retourné des zones, on découpe le texte brut en sections
    if (zones && Object.keys(zones).length > 0) {
      const plainText = rawText; // peut contenir des <em> si text_highlight
      // Collecter tous les segments avec leur zone et position
      type Seg = { zone: string; start: number; end: number };
      const allSegs: Seg[] = [];
      for (const zoneName of ZONE_ORDER) {
        const segs = zones[zoneName];
        if (!Array.isArray(segs)) continue;
        for (const seg of segs) {
          allSegs.push({ zone: zoneName, start: seg.start, end: seg.end });
        }
      }
      allSegs.sort((a, b) => a.start - b.start);

      if (allSegs.length > 0) {
        // Regrouper les segments par zone dans l'ordre
        const sectionsMap: Map<string, string[]> = new Map();
        for (const seg of allSegs) {
          const slice = plainText.slice(seg.start, seg.end);
          if (!sectionsMap.has(seg.zone)) sectionsMap.set(seg.zone, []);
          sectionsMap.get(seg.zone)!.push(slice);
        }

        const sectionHtmlParts: string[] = [];
        // Texte avant la première zone
        const firstStart = allSegs[0].start;
        if (firstStart > 0) {
          const preamble = plainText.slice(0, firstStart).trim();
          if (preamble) {
            sectionHtmlParts.push(
              `<section class="jur-section jur-section--decision-text jur-zone jur-zone--preamble">` +
              textSegmentToHtml(preamble) + `</section>`
            );
          }
        }

        for (const zoneName of ZONE_ORDER) {
          const parts = sectionsMap.get(zoneName);
          if (!parts || parts.length === 0) continue;
          const label = ZONE_LABELS[zoneName] || zoneName;
          const inner = parts.map(p => textSegmentToHtml(p.trim())).join('');
          sectionHtmlParts.push(
            `<section class="jur-section jur-section--decision-text jur-zone jur-zone--${zoneName}">` +
            `<h2 class="jur-zone-title"><span class="jur-zone-label">${label}</span></h2>` +
            inner + `</section>`
          );
        }

        texteHtml = sectionHtmlParts.join('\n');
      } else {
        // zones vides : fallback paragraphes
        texteHtml = `<section class="jur-section jur-section--decision-text">` +
          textSegmentToHtml(rawText) + `</section>`;
      }
    } else {
      // Pas de zones : fallback avec paragraphes (déjà mieux que le bloc plat)
      texteHtml = `<section class="jur-section jur-section--decision-text">` +
        textSegmentToHtml(rawText) + `</section>`;
    }
  }
  return {
    format: 'html',
    body: [
      `<h1 class="jur-doc-h1">${decisionTitle}</h1>`,
      meta, procedureHtml, titrageHtml, themesHtml, sommaireHtml, texteHtml,
      rapprochementsHtml, visaHtml, filesHtml,
    ].filter(Boolean).join('\n'),
  };
}
