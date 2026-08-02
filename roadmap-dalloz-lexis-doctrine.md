# Feuille de route — vers un niveau Dalloz / LexisNexis / Doctrine

*Document de référence, à ressortir après la phase de stabilisation.*

> **Le point le plus important à retenir de ce document : la découverte la
> plus rentable est déjà dans vos mains, sans le savoir.**
> En creusant le schéma `decisionFull` de Judilibre pour corriger le bug de
> date des décisions, on est tombés sur des champs jamais exploités —
> `contested`, `forward`, `rapprochements`, `timeline` — qui constituent déjà,
> côté API, l'embryon exact du graphe de citations que Doctrine.fr met en
> avant comme argument de vente principal. C'est de la donnée gratuite, il ne
> manque que l'interface. **Priorité n°1 du document — cf. A1 ci-dessous.**
> *(Statut : implémenté le 05/07/2026 — voir `extract-judilibre.ts`.)*

## Comment lire ce document

Les pistes sont classées en 3 catégories selon leur **coût réel**, pas leur
attrait :

- **A — Données déjà dans l'API, non exploitées.** Le plus rentable : pas de
  nouvelle source de données à intégrer, juste de l'UI/UX à construire sur des
  champs qu'on a déjà découverts en corrigeant les bugs de cette conversation.
- **B — Ingénierie nouvelle, faisable avec l'architecture actuelle.** Demande
  du développement réel (backend + frontend) mais rien d'exotique.
- **C — Hors de portée sans partenariat de contenu.** Ce que Dalloz/Lexis
  vendent est en bonne partie de la **doctrine rédigée par des juristes**
  (commentaires, encyclopédies, fiches pratiques) — pas des données publiques.
  Impossible à répliquer avec Légifrance/Judilibre seuls.

---

## A — Déjà dans l'API, à exploiter

### A1. Graphe de citations Judilibre (le plus gros gisement inexploité) — ✅ fait
En creusant le schéma `decisionFull` pour corriger le bug de date/sommaire,
on a découvert que Judilibre expose **déjà** un embryon du graphe de
citations que Doctrine.fr met en avant comme fonctionnalité phare :

- `contested` → la décision attaquée par ce pourvoi
- `forward` → les décisions qui **citent celle-ci** par la suite
- `rapprochements` → décisions similaires/rapprochées
- `timeline` → l'historique procédural de l'affaire

Implémenté : panneau « Procédure » (contested/forward/timeline, en tête de
document) + sections « Rapprochements de jurisprudence », « Textes
appliqués » (`visa`) et « Documents associés » (`files`) en fin de document,
façon annexes. Rapprochements et textes appliqués s'ouvrent directement dans
l'application (mécanisme `data-cite-id` existant) plutôt que de renvoyer vers
le site public : pour les rapprochements, dès que `decisionLink.id` est un
identifiant JUDILIBRE (hash 24 hex) ; pour les textes appliqués, en extrayant
l'identifiant LEGIARTI/LEGITEXT porté par `textLink.url`
(`affichCodeArticle.do?idArticle=…`) — `textLink.id` lui-même est un entier
interne JUDILIBRE sans route de résolution documentée, inutilisable pour la
navigation. Repli en lien externe uniquement quand aucun identifiant n'est
extractible. Au passage, correction d'un bug de routage de navigation par
citation (`useDocumentViewer.openDocument`) qui aurait fait échouer les
nouveaux liens `contested`/`forward`/`rapprochements` dès que le filtre de
recherche courant n'était pas déjà sur JUDILIBRE : la détection de la source
se fait maintenant par la forme de l'identifiant (hash 24 hex), pas par
l'onglet de recherche actif.

#### Ce qu'on cherche à reproduire : le *legal graph* Doctrine

Doctrine documente sa propre vision de ce mécanisme dans un article de blog
([« Le legal graph, ou comment rendre données et liens juridiques
exploitables »](https://blog.doctrine.fr/le-legal-graph-ou-comment-rendre-donnees-et-liens-juridiques-exploitables/),
mai 2022). À retenir pour situer notre implémentation actuelle par rapport à
leur cible, et identifier ce qui reste hors de portée sans travail
d'enrichissement propre :

- **Un lien y est un objet qualifié, pas une simple flèche.** Deux citations
  d'une même décision antérieure n'ont pas la même valeur selon qu'il s'agit
  du fil procédural de l'affaire (même sens que notre `contested`/`forward`)
  ou d'une citation jurisprudentielle d'un arrêt de principe (plus proche,
  côté valeur, de nos `rapprochements` — mais Doctrine va plus loin en
  *typant* explicitement la nature du lien, ce que l'API JUDILIBRE ne nous
  donne pas : `rapprochements` est une liste plate, sans distinction entre
  « décision de référence » et « affaire simplement proche »).
- **Construction en deux temps :** une partie des liens vient directement des
  institutions (chez nous : exactement ce que JUDILIBRE/Légifrance nous
  fournit déjà, cf. A1/A2/A4) ; le reste est produit par du texte-mining —
  détection d'une référence dans un document, puis résolution de l'entité
  visée (ex. désambiguïser un avocat homonyme via des indices contextuels).
  Cette seconde brique n'existe pas chez nous et suppose un travail de NLP
  dédié, indépendant de toute nouvelle source de données — donc plutôt une
  piste **B/C** qu'un prolongement direct de A1.
- **Quatre usages qu'ils tirent du graphe, à garder en tête pour prioriser la
  suite :**
  1. *Contextualiser* — aperçu du contenu d'un article cité directement
     depuis la décision qui le cite (chez nous : embryon avec la bulle de
     prévisualisation au survol, `handleLinkMouseEnter`, déjà en place et
     réutilisable telle quelle pour les nouveaux liens de graphe).
  2. *Améliorer la pertinence de recherche* — un texte cité par de nombreuses
     décisions gagne en pertinence documentaire, et le vocabulaire des
     décisions qui le citent enrichit sa indexation (leur exemple : un article
     du code du travail ne mentionne jamais littéralement le « Pass Navigo »,
     mais les décisions qui l'appliquent si — ce qui le fait remonter sur
     cette requête). Suppose d'indexer nous-mêmes ce croisement décision ↔
     texte appliqué ; hors de portée immédiate (nécessite un moteur de
     recherche propre plutôt qu'un simple relai vers les API Légifrance/
     JUDILIBRE) — à rapprocher de B2.
  3. *Recommander/alerter* — suivre un texte ou une affaire et être notifié
     des décisions liées à venir : c'est exactement B3 (veille juridique),
     déjà présent dans ce document, maintenant qu'on a la donnée de graphe
     pour l'alimenter.
  4. *Naviguer en entonnoir* — partir d'une jurisprudence, remonter au texte
     de loi fondateur, puis à l'amendement qui explique une notion : c'est la
     promesse concrète de A1 une fois combiné à A2 (renvois d'articles) et à
     A4 (dossiers législatifs, où vivent les amendements/exposés des motifs).
- **Limite qu'ils reconnaissent eux-mêmes :** ils n'exploitent en pratique que
  des liens « de degré 1 » (directs) ; les liens de degré 2 (transitifs —
  leur exemple : un article de loi modifie un article du CGI, une doctrine
  administrative cite la loi sans citer l'article du CGI qu'elle modifie
  pourtant en pratique) restent un chantier ouvert même pour eux. Point utile
  pour calibrer nos ambitions : notre graphe JUDILIBRE actuel (A1) est déjà
  strictement du degré 1 fourni tel quel par l'API — inutile de viser plus
  loin que Doctrine elle-même sur ce point sans investissement NLP dédié.

En résumé : A1 nous donne la **brique de données** et l'**infrastructure de
navigation** du legal graph (liens degré 1, contextualisation au survol,
funnel de navigation) — ce qui manque pour rejoindre l'ambition Doctrine,
c'est la **qualification sémantique des liens** et l'**enrichissement par
texte-mining**, qui sont hors de portée sans un chantier NLP dédié (à ajouter
en catégorie C tant qu'aucune source de données/algorithme propre n'est
engagée).

### A2. Historique et renvois d'articles (Légifrance)

**Partie historique + modifications — ✅ fait (05/07/2026).** `Article.historique`
et `Article.lienModifications` (nom réel du champ sur `/consult/getArticle` —
à ne pas confondre avec `ConsultArticle.lstLienModification`, nom du champ
équivalent sur le schéma voisin utilisé quand l'article est imbriqué dans un
texte via `/consult/legiPart` et consorts) étaient déjà dans le payload
récupéré par le frontend et jamais rendus. Implémenté : section « Modifié
par » (liens vers l'article modificateur précis via `articleId`, repli sur le
texte modificateur entier via `textCid` si absent) et section « Historique »
(texte HTML, avec les mêmes liens internes que le corps de l'article) sur la
fiche article dédiée ; version compacte de « Modifié par » sur les articles
imbriqués d'un texte entier. Zéro appel réseau supplémentaire. Au passage,
correctif d'un bug de routage de navigation par citation entre fonds
Légifrance eux-mêmes (CODE/LODA/JORF) — même classe de bug que celui déjà
corrigé pour Judilibre en A1, mais resté invisible jusqu'ici faute de lien
traversant les fonds : `LienModification.textCid` peut désormais pointer vers
un fond différent de l'article consulté (ex. un JORFTEXT modifiant un article
de code) et l'aurait fait échouer silencieusement sans ce correctif.

**Partie graphe de citations — reste à faire, effort réel de type B.**
`ConsultArticle.lstLienCitation` est documenté « Toujours vide (voir
propriété `comporteLiens`) » : le graphe « cet article cite / est cité par »
nécessite un nouvel appel backend (`POST /consult/relatedLinksArticle`), pas
encore câblé côté serveur. Contrairement à la partie historique/modifications
ci-dessus, ce n'est pas une simple UI sur donnée déjà en main.

### A3. Table des matières de code, navigation complète — ✅ fait
La structure `sections`/`articles` récursive existe déjà (`renderSections`) ;
on pourrait en faire un vrai plan de code interactif dans la colonne latérale
(actuellement, seule la table des matières du document ouvert est visible),
avec navigation façon sommaire Dalloz.
*(Statut : implémenté le 06/07/2026 — plan arborescent avec plier/déplier et
scroll-spy révélant le chemin courant, cf. `useDocumentViewer.ts`
(`buildTocTree`/`revealTocPath`) et `JuridiquePanel.tsx` (`TocTreeList`).
NB : reste bloqué par une limitation côté API PISTE/DILA pour les codes les
plus volumineux — Code civil, Code de la défense — qui échouent avant même
d'atteindre cette étape, cf. `routes.rs` : `/consult/code/tableMatieres` ET
son repli `/consult/legi/tableMatieres` renvoient tous deux HTTP 400
« L'expression à valider est fausse » sur ces textes, sans repli connu côté
API pour charger la structure par sections.)*

### A4. Dossiers législatifs — ✅ fait (07/07/2026, schéma vérifié le 08/07/2026)
`SearchResult.dossiersLegislatifs` (travaux préparatoires d'une loi) est
présent dans le schéma de recherche mais n'apparaît nulle part dans l'UI —
c'est pourtant un classique des bases juridiques pro (exposés des motifs,
débats parlementaires liés à un texte).

Implémenté : badge « 📜 Travaux prépa. » sur les cartes de résultat LODA/JORF
qui exposent ce champ, et bouton équivalent dans l'en-tête de la visionneuse
une fois le texte ouvert. Ouverture via `GET /dossier/:id`, qui existait déjà
côté backend (`routes.rs::get_dossier` / `legifrance.rs::get_dossier_legislatif`)
mais n'était appelé par aucun code frontend. Nouveau module d'extraction
dédié : `content/extract-dossier-legislatif.ts`.

Schéma vérifié le 08/07/2026 contre le swagger officiel Légifrance v2.4.2
fourni par l'utilisateur (définitions `ConsultDossierLegislatifResponse` /
`DossierLegislatif` / `Dossier` / `Arborescence` / `Niveau` / `Lien` /
`Legislature`), après une première version écrite sur la base d'une
reconstitution partielle (recherche web, pas d'accès à un compte PISTE) —
plusieurs noms de champs devinés initialement étaient incorrects et ont été
corrigés :
- Réponse enveloppée sous la clé `dossierLegislatif` (pas à la racine).
- Pas de liste "actes législatifs" à plat : arborescence récursive
  (`arborescence.niveaux[].liens[]`, `niveaux[].niveaux[]…`), un niveau par
  chambre, liens externes uniquement (Sénat/AN/JO).
- `exposeMotif` (HTML), pas `exposeDesMotifs`.
- `dossiers[]` = sous-dossiers avec contenu HTML propre (`contenuDossier`),
  pas des documents PDF.
- Le champ `dossiersLegislatifs` est présent à la fois sur `SearchResult` ET
  directement sur la réponse de consultation du texte
  (`ConsultTextResponse`/`ConsultJorfResponse`) — et contient déjà des objets
  `DossierLegislatif` complets, pas de simples id/titre. `useDocumentViewer.ts`
  lit désormais `raw.dossiersLegislatifs` en priorité (repli sur le résultat
  de recherche sinon) plutôt que de ne compter que sur le report depuis la
  recherche.

---

## B — Ingénierie nouvelle, réaliste avec l'architecture actuelle

### B1. Comparateur de versions côte à côte (déjà évoqué)
On a construit la brique de base (liste des versions, navigation). Il manque
la vraie vue à deux volets façon Légifrance/Dalloz, avec sélection libre des
deux versions à comparer — pas juste une navigation séquentielle.

### B2. Recherche unifiée cross-fonds
Aujourd'hui, on cherche fonds par fonds. Un moteur qui interroge plusieurs
fonds en parallèle et fusionne les résultats par pertinence (texte + JP +
doctrine si on l'a un jour) est ce qui distingue une recherche « pro » d'une
recherche « documentaire ».

### B3. Veille juridique / alertes
Suivre un texte, un thème ou une requête sauvegardée, et être notifié des
nouvelles décisions/modifications. Nécessite : stockage des critères de veille
par utilisateur, un job planifié côté backend qui repasse périodiquement les
recherches, et un canal de notification (email, ou fil dans l'appli).

### B4. Export et mise en forme professionnelle
Export Word/PDF d'un dossier avec mise en forme de citation juridique
standard (références neutres, bibliographie), au-delà du simple export
Markdown actuel.

### B5. Dossiers collaboratifs
Partage d'un dossier entre plusieurs utilisateurs d'un même cabinet/service,
avec annotations partagées — suppose un modèle multi-utilisateurs (actuellement
les dossiers/favoris sont en `localStorage`, donc strictement individuels et
liés à un navigateur).

### B5bis. Conserver les réponses LLM d'un dossier pour rappel — ✅ fait
*(Noté le 06/07/2026, implémenté le 07/07/2026.)*
Chaque analyse (synthèse/points clés/contradictoire) est maintenant rattachée
au dossier lui-même (`Dossier.analyses: DossierAnalysis[]`, cf. `types.ts` et
`useDossiers.ts`), horodatée et conservée en historique (plafonné aux 20
dernières par dossier) plutôt qu'écrasée à chaque nouvelle demande — pensé dès
le départ pour accumuler plusieurs analyses dans le temps. Un panneau
repliable « Historique des analyses » dans `JuridiquePanel.tsx` permet de
revoir (`reopenDossierAnalysis` → `onShowInChat`, qui réaffiche le texte tel
quel SANS relancer le LLM) ou supprimer une entrée. Persisté en `localStorage`
comme le reste des dossiers — donc toujours strictement individuel/local
(cf. B5 : pas encore multi-utilisateurs).

### B6. Historique de consultation et suggestions
« Documents consultés récemment », « autres textes souvent consultés avec
celui-ci » — nécessite de collecter et agréger des données d'usage (avec les
questions de confidentialité que ça soulève, à trancher en amont).

### B7. IA juridique approfondie
Le bouton « Analyser avec l'IA » existe déjà comme point d'entrée. On pourrait
aller plus loin : question-réponse sur un corpus de dossier, génération de
synthèses comparatives entre plusieurs décisions, détection de textes
pertinents à partir d'une situation de fait décrite en langage naturel.

---

## C — Hors de portée sans partenariat de contenu

- **Commentaires doctrinaux, encyclopédies, fiches pratiques** : c'est le
  cœur du modèle économique Dalloz/Lexis (juristes qui rédigent de la
  doctrine). Aucune API publique ne fournit ça — il faudrait soit produire ce
  contenu en interne, soit licencier du contenu tiers.
- **Formation continue / e-learning** : proposé par Lexis, hors périmètre
  d'un outil de recherche documentaire.
- **Certification/opposabilité de la veille** (utilisée en cabinet pour
  justifier d'avoir été informé d'un changement réglementaire) : possible en
  théorie (B3 + horodatage), mais la valeur juridique d'une telle attestation
  dépasse le sujet technique.
- **Qualification sémantique des liens du graphe + enrichissement par
  texte-mining** (cf. « legal graph » Doctrine, note sous A1) : détecter et
  typer des liens qui n'existent dans aucune API publique (ex. distinguer
  « décision de référence » vs « affaire simplement proche » dans les
  rapprochements, ou relier deux textes qui se citent transitivement sans
  jamais se référencer directement) suppose un pipeline NLP propre — détection
  d'entités + désambiguïsation — et non plus seulement de l'UI sur des champs
  déjà fournis. Hors de portée tant qu'aucun chantier NLP dédié n'est engagé ;
  à ne pas confondre avec A1 (liens degré 1 bruts, déjà exploitables tels
  quels).

---

## Priorisation suggérée (à discuter)

1. **A1 (citations Judilibre)** — rapport effort/valeur imbattable : la donnée
   existe déjà, c'est de l'UI pure, et c'est la fonctionnalité la plus
   identifiable comme « Doctrine-like ».
2. **A2/A3/A4** — même logique, données déjà là, périmètre plus restreint.
3. **B1 (comparateur de versions)** — on a déjà la moitié du chemin.
4. **B2 (recherche unifiée)** — gros chantier mais changement d'image fort
   (« outil de recherche » plutôt que « série de fonds séparés »).
5. **B3/B4/B5/B6/B7** — à prioriser selon les retours utilisateurs une fois
   les premiers points livrés.

---

*Document vivant — à mettre à jour au fil des découvertes sur les API et des
retours utilisateurs.*
