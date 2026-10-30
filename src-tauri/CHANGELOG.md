# Changelog

Toutes les modifications notables de ce projet sont documentées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/),
et ce projet adhère au [Versioning Sémantique](https://semver.org/lang/fr/).

---

## [Non publié]

### Ajouté

- **Fiche d'une décision de justice** (`GET /api/juridique/decision/:id/fiche`, module `jurisprudence.rs`) :
  identité, niveau (juge du fond / Cour de cassation) et rapprochements fournis par les API — décision attaquée
  (`contested`), décision suivante (`forward`), rapprochements de jurisprudence, dates clés, visas (Judilibre) ;
  décision attaquée et liens typés `liens` (Légifrance, `typeLien` et `sens` restitués tels quels) — chacun avec
  sa provenance et sa chronologie (antérieure / postérieure, d'après les dates). Les « cité par » et les
  revirements, absents des deux API, sont listés comme non fournis. Le badge d'une décision vérifiée porte un
  bouton ⛓ qui ouvre la fiche ; `decision_id` est ajouté à chaque vérification de décision retrouvée.
  Test réel facultatif : `live_piste_fiche`.
- **Vérification des citations — ECLI, Conseil d'État, juridictions administratives, cours d'appel** : les
  décisions citées par ECLI, par numéro de requête (Conseil d'État, cours et tribunaux administratifs) ou par
  numéro de RG (cours d'appel) sont recherchées (Judilibre, jurisprudence administrative de Légifrance) et
  reçoivent un statut visible : ✅ retrouvée (juridiction, formation, date, solution, publication, source dans
  l'info-bulle), ⚠️ date ou chambre différente, ❓ non retrouvée (≠ inexistante), ⚪ juridiction non interrogée.
  Une décision n'est « vérifiée » que si son numéro ou son ECLI correspond à celui cité : une décision voisine
  renvoyée par la recherche est écartée. Fond CETAT : recherche par le champ `NUM_DEC` (les champs `NUM_AFFAIRE`
  et `ECLI` n'y existent pas), décision reconnue par le titre du résultat (juridiction, date, numéro), lecture
  `/consult/juri` facultative. Un RG, partagé par plusieurs cours, n'est retenu que si la date citée ou le siège
  cité (filtre `location` de Judilibre) le désigne. Un ECLI retrouvé sous son numéro dans les sources fournies au modèle n'est plus signalé
  « hors sources ». Nouveau module `verify_juris.rs` ; test réel `live_piste_juridictions` (ignoré par défaut).
- **Documentation** : `docs/CHECKLIST_RELEASE.md`, `docs/MCP_PACK_SIGNATURE.md` et `docs/MCP_AUTORISATION.md`
  (référencés par le README, le CHANGELOG et le workflow de release, mais absents).
- **CI** : Dependabot (cargo, npm, GitHub Actions) ; l'audit des dépendances ne bloque plus les PR et ouvre
  une issue lors de l'exécution hebdomadaire.
- **Droit dans le temps — « Droit applicable au… »** : une date des faits peut être fixée (puce au-dessus de la zone de
  saisie du chat, champ dans la recherche des codes). Elle est transmise au modèle (il applique le droit en vigueur à
  cette date et dit quand il ne l'a pas vérifié), la vérification des citations indique pour chaque article cité la
  version applicable à la date, les modifications intervenues depuis (🕘) ou son absence à cette date (🚫), et le lien
  ouvre la version applicable. La recherche d'articles et de phrases dans les codes renvoie les versions en vigueur à
  la date. Calcul fait par le code à partir de la liste des versions Légifrance (`articleVersions`), jamais par le
  modèle. La date n'est pas conservée d'une session à l'autre. Nouveau module `temporal.rs`.
- **Visionneuse juridique** : bouton « Copier » dans la bulle qui apparaît à la sélection d'un texte (à côté
  de « Annoter »), pour copier la sélection dans le presse-papiers.

### Corrigé

- **Fiche article — versions** : sous la barre « Versions de cet article », un bloc gris affichait le texte d'une
  AUTRE version (le plus souvent la version actuelle) au-dessus de la rédaction réellement affichée, ce qui
  prêtait à confusion. Les pastilles de versions sont des liens de navigation : plus aucun texte n'y est inséré.
  Les dates antérieures à 1970 s'affichent désormais (la date de début des versions d'articles de 1804 était
  omise). Quand une date des faits est fixée, un bandeau indique si la version affichée était celle en vigueur à
  cette date (et, sinon, invite à choisir la version applicable).
- **Droit applicable au… — réponse du chat** : le modèle recopiait le texte renvoyé par l'outil Légifrance (la
  version actuelle) sans tenir compte de la date des faits, la consigne de reproduction fidèle des outils
  l'emportant sur la consigne de date. Désormais, pour les articles de code cités dans les derniers messages de
  l'utilisateur, Juridix lit lui-même sur Légifrance la rédaction applicable à la date et la fournit au modèle comme
  texte faisant foi ; la consigne de date est placée après celle des outils, et un rappel est ajouté à la dernière
  question dans la charge envoyée au modèle (jamais affiché). Pour une question sans article cité, seuls la
  consigne et le rappel s'appliquent : la vérification des citations signale alors un éventuel écart.
- **Droit applicable au… — citations textuelles** : avec une date des faits, une citation rattachée à un article
  est comparée à la rédaction applicable à cette date, et non plus au texte actuel (qui la faisait juger
  « inexacte » alors qu'elle reproduisait fidèlement l'ancienne version). Cas distincts : passage retrouvé dans la
  rédaction de la date ✅ ; passage qui ne figure que dans la version actuelle (la citation ne reflète pas le droit
  à la date des faits) ; article non en vigueur à cette date ; rédaction à la date illisible (« non vérifiable »,
  jamais un verdict sur le texte actuel).
- **Pack MCP — script de signature** : le script `scripts/sign-mcp-pack-release.sh` est **supprimé**. Il signait avec un
  commentaire « juridix-mcp-pack v0.3.4 » alors que Juridix exige le mot `version:0.3.4` : une release signée avec lui
  aurait été refusée à l'installation. La signature d'une release du pack se fait avec `sign-mcp-pack.sh`, conservé
  dans le seul dépôt du pack (`demeter-mcp-pack`) et partagé avec Demeter ; voir `docs/MCP_PACK_SIGNATURE.md`.
- **Droit applicable au… (calendrier)** : la date n'est appliquée qu'à la validation (« Appliquer » / Entrée) ;
  changer une année ou un mois ne ferme plus la fenêtre et ne relance plus la vérification des citations à
  chaque clic. Années limitées à 1800–2200 ; vérifications simultanées plafonnées à 3.
- **Légifrance — recherche dans les codes** : « code civil, article 1140 » (et variantes : « art. 1140 C. civ. »,
  « 1140 code civil », « L. 1234-5 code du travail », « articles 1103, 1104 et 1105 du code civil ») retourne
  désormais l'article demandé, ouvert directement. Les phrases libres (tous les codes ou un code donné) sont
  cherchées dans le contenu des articles par paliers — expression exacte, puis tous les mots, puis au moins un
  des mots — avec une note indiquant le palier retenu. « code civil » seul et les champs TITLE/TABLE gardent
  leur comportement. Nouveaux modules `code_query.rs` / `code_search.rs`.

---

## [1.1.5] — 2026-10-23

### Sécurité

- **Pack MCP** : la signature minisign de `checksums.txt` est vérifiée avec la clé publique
  du fichier `src-tauri/mcp-pack.pub` — **la même clé et la même signature que Demeter** (commentaire
  signé `demeter-mcp-pack version:<version>`, produit par `sign-mcp-pack.sh`). La variable
  `JURIDIX_MCP_PACK_PUBKEY` est supprimée. Une build de release ne compile que si `mcp-pack.pub`
  contient une clé ; le mode de transition « SHA-256 seul » n'existe plus dans les binaires publiés.
  L'empreinte SHA-256 du binaire installé est enregistrée dans le trousseau système et revérifiée
  avant chaque lancement (échec fermé). Les installations existantes doivent réimporter les serveurs
  une fois.
- **ECharts** : les blocs `echarts` ne sont plus évalués par `new Function` mais analysés
  par `JSON.parse` (`frontend/src/utils/echartsOption.ts`), avec validation de l'arbre et retrait des
  balises HTML des chaînes.

### Ajouté

- **Longueur maximale des réponses** (Configuration → LLM) : menu simple (Automatique, Courte,
  Standard, Longue, Très longue) qui plafonne les tokens de sortie du modèle. Utile avec une fenêtre de
  contexte de 128K : le plafond automatique (`LLM_MAX_TOKENS`, 100 000 par défaut) peut faire refuser par
  le serveur une longue analyse. Le choix est transmis à chaque requête de chat (`max_tokens`, borné entre
  256 et 1 000 000 côté serveur) ; en mode automatique, `LLM_MAX_TOKENS` reste la valeur appliquée.

- **Vérification des citations étendue** : en plus des articles de codes (inchangés),
  - **décisions de la Cour de cassation** citées par leur numéro de pourvoi (« Cass. soc., 12 mars 2024,
    n° 22-12.345 ») : recherche dans Judilibre ; l'info-bulle indique chambre, date, solution et publication ;
    une date ou une chambre qui ne concordent pas avec celles citées donnent ⚠️ ; une décision non retrouvée
    donne ❓ (et non ❌ : Judilibre n'est pas exhaustif) ;
  - **lois, ordonnances et décrets** cités par leur numéro : existence et état juridique (fonds LODA),
    cohérence de la date citée (auparavant : ancrage seul) ;
  - **conventions collectives** citées par leur IDCC (consultation KALI) : existence seulement, car le
    conteneur d'une convention n'a pas d'état juridique dans Légifrance (seuls ses textes en ont un) ;
  - **citations textuelles** : un passage entre guillemets rattaché sans ambiguïté à un article de code est
    comparé au texte de l'article sur Légifrance (✅ identique, ⚠️ proche, ⚠️ absent) ; les autres passages
    sont comparés aux documents fournis au modèle, avec un verdict uniquement positif (✅ ou ⚠️ proche).
    La comparaison ignore la casse, la ponctuation, les apostrophes typographiques, les ellipses et les
    insertions entre crochets ;
  - une citation est rattachée à l'article nommé dans la **phrase d'introduction** qui la précède, même
    séparée par une ligne vide (« … dispose que : » puis un bloc de citation), et les **blocs de citation
    Markdown** (`> …`) sont comparés au texte de l'article même sans guillemets ; une étiquette en tête
    (« **Article 1140** ») n'entre pas dans la comparaison. Un « et actuel » ajouté au texte officiel donne
    ainsi ⚠️ « proche mais non identique » ;
  - pour une citation « proche » du texte de l'article, l'info-bulle indique **ce qui diffère** : mots
    ajoutés (« et actuel ») et mots du texte officiel omis, par alignement mot à mot ;
  - les citations entre **guillemets droits** `"…"` sont reconnues (en plus de « » et “ ”), et quand la
    réponse ne cite elle-même aucun article de code, l'article de la **question** (« Cite mot pour mot
    l'article 1240 du Code civil ») sert de référence : une citation absente de cet article donne ⚠️, sauf
    s'il y a plusieurs citations (on ne sait alors pas laquelle concerne l'article) ;
  - l'ancrage (🔎 *hors sources*) s'applique aussi aux décisions et aux IDCC ;
  - sans identifiants PISTE, ces vérifications se dégradent en silence (ancrage et comparaison aux
    documents restent actifs) ; seul un article de code continue d'exiger PISTE.
  Un test manuel contre la vraie API (`cargo test --lib live_piste -- --ignored`) est fourni pour valider
  les recherches par numéro avant chaque release.

### Modifié

- **Synthèse de veille** : un texte trop long est désormais réduit en conservant son **début et sa fin**
  (60 % / 40 %) et en omettant le milieu, au lieu de ne garder que le début : le dispositif d'une décision
  et les dispositions d'entrée en vigueur d'un décret restent visibles. Les plafonds passent de 15 000 à
  40 000 caractères par texte et de 90 000 à 240 000 au total (≈ 70 000 jetons, compatible 128K). Le prompt
  demande au modèle de ne pas conclure à l'absence d'une information dans un texte tronqué et de signaler
  qu'il est partiel ; le bandeau et la note finale indiquent que le milieu n'a pas été transmis.
- **Modèle** : le champ « Modèle par défaut » est supprimé de la configuration. Il bloquait
  l'enregistrement pour qui ne savait pas quoi y saisir ; seuls l'endpoint et le token sont requis, et le
  modèle se choisit dans la liste récupérée auprès de l'endpoint (menu en haut de l'écran). Une valeur déjà
  enregistrée par une version antérieure reste utilisée comme modèle préféré. Si l'endpoint ne renvoie aucune
  liste de modèles, le badge « Aucun modèle disponible » l'indique et l'envoi d'un message est refusé avec
  une explication.

### Corrigé

- **Faux sourcing dans le chat sans outils** : le prompt de l'espace « Juris » annonce un accès à Légifrance
  et Judilibre via MCP. Quand aucun serveur MCP n'est actif, le modèle jouait cet accès (« Texte officiel
  (à jour)… Source : Légifrance, version consolidée au… ») en citant de mémoire, parfois à tort. Un rappel
  est désormais ajouté au message système lorsqu'aucun outil n'est exposé : ne pas affirmer avoir consulté
  une base, signaler les citations de mémoire, résumer plutôt que citer entre guillemets en cas de doute.
- **Textes d'outils recopiés de façon infidèle** : quand des outils MCP sont actifs, le message système
  demande désormais de reproduire à l'identique les textes qu'ils renvoient (sans les compléter de mémoire).
  Le résultat brut d'un outil est journalisé au niveau `debug` (`RUST_LOG=juridix_lib=debug`) pour
  distinguer une donnée erronée d'un texte altéré par le modèle.
- **Consultation d'une convention collective par IDCC** (`/consult/kaliContIdcc`) : la requête envoyait
  la propriété `idcc` alors que la spécification Légifrance attend `id` (« identifiant ou numéro IDCC »).
- **Recherche Judilibre** : la facette « Conseil d'État » (`ce`) provoquait une erreur HTTP 400 de PISTE
  (Judilibre ne couvre que l'ordre judiciaire : `cc`, `ca`, `tj`, `tcom`, `cph`). Elle est retirée, et les
  tribunaux de commerce et conseils de prud'hommes sont ajoutés. Les décisions du Conseil d'État se
  recherchent dans Légifrance, fond « Jurisprudence admin. (CE) ».
- **Version de l'application** : `tauri.conf.json` était resté en 1.1.4, les installeurs auraient été
  numérotés 1.1.4. Le README (titre et badge) est aligné sur la 1.1.5.
- **Pack MCP** : le refus d'un chemin absolu dans l'archive est explicite (zip 9 retire le `/` initial au
  lieu de le rejeter). Les restes de la variable `JURIDIX_MCP_PACK_PUBKEY` (workflow de release,
  `build.rs`, README) sont supprimés.

### Dépendances

- **Frontend** : `echarts` 6.1, `katex` 0.19 (forcé pour `mermaid`, `rehype-katex` et `remark-math`
  via `overrides`) et `dompurify` 3.4.17 (indirect, via `mermaid`) ; `npm audit` ne signale plus rien.
- **Rust** : `tokio` 1.54.1 et `anyhow` 1.0.104 (avis « unsound »), `rustls` 0.23.46, `rustls-webpki` 0.103.16, `webpki-roots` 1.0.10, `ctutils` 0.4.5 et
  `zip` 9 (la lecture du nom d'entrée est adaptée ; les chemins absolus sont refusés explicitement
  à l'extraction du pack MCP).
- **Audit** : l'avis RUSTSEC-2026-0387 (`crypto-bigint` 0.5.5, via `jsonwebtoken` → P-256/P-384) est ignoré
  dans `src-tauri/.cargo/audit.toml` avec sa justification : seul le HMAC (HS256) est utilisé, jamais
  l'ECDSA.
- **CI** : l'étape d'audit n'est plus informative (`continue-on-error` retiré) ; `cargo fmt` appliqué.

---

## [1.1.4] — 2026-10-12

### À savoir en mettant à jour

- **Serveurs MCP HTTP déjà configurés** : ils ne sont plus utilisés par le modèle tant qu'ils n'ont pas
  été **autorisés une fois** (badge « ⏳ à autoriser » dans les réglages ; bannière au premier message).
  Les serveurs officiels (Légifrance, Judilibre) ne sont pas concernés. Voir `docs/MCP_AUTORISATION.md`.
- **Plus d'images** dans les réponses : le texte alternatif s'affiche à la place (Juridix ne contacte plus
  aucun hôte web pour afficher une image).
- **Secrets** : si le trousseau du système est indisponible (fréquent sous Linux sans Secret Service),
  les identifiants ne sont plus écrits sur le disque : ils sont ressaisis à chaque lancement, une bannière
  l'explique. Les anciens résidus en clair sont migrés vers le trousseau puis supprimés.
- **Sauvegarde `.pre-encryption.bak`** (si une ancienne base en clair a été migrée) : à supprimer dans
  Réglages → *Données et confidentialité* une fois la migration vérifiée.
- **Pack MCP** : la signature des releases est prévue mais **non exigée dans cette version** (aucune
  clé publique compilée) ; un retour à une version antérieure du pack est désormais refusé.
- **Polices et styles embarqués** : apparence identique, plus aucune connexion à Google Fonts ni jsDelivr.

### Ajouté

- **Export et effacement RGPD** — Réglages → *Données et confidentialité* :
  export de toutes les données en archive `.zip` (conversations en JSON et en Markdown lisible,
  veilles, dossiers, favoris, annotations, historique, réglages d'interface ; chiffrement AES-256
  facultatif ; jamais d'identifiants), suppression de la sauvegarde en clair laissée par la migration
  vers SQLCipher, et « Tout effacer » (base, prompts, trousseau, `localStorage`, sauvegarde) avec
  confirmation par saisie de `EFFACER`. Effacement en deux temps, repris au démarrage suivant en cas
  d'arrêt brutal ; commandes IPC uniquement (hors API HTTP)
- **Synthèse de veille** — dans la veille, on coche des textes ou décisions (15 au
  maximum), éventuellement avec une consigne (« pour un DRH… »), puis « Synthétiser » :
  le texte intégral de chacun est chargé depuis Légifrance / Judilibre, le modèle
  rédige une note de veille (synthèse générale, textes par catégorie, points
  d'attention) sans aucun outil (restitution factuelle des seuls textes fournis), et
  la note — complétée de la liste exacte des textes analysés, des textes tronqués ou
  non chargés — se télécharge en fichier Word. Nouveau mode `veille_synthese` de
  `/api/juridique/envoyer-chat` (réutilise le préfiltre Winnow et le chat)
- **Vérification des citations du LLM** — les références d'articles de codes citées dans
  une réponse sont vérifiées sur Légifrance (✅ en vigueur / ⚠️ abrogé ou plus en vigueur /
  ❌ introuvable), en arrière-plan, dans la bulle de chat et l'onglet « Réponse complète »
  des artéfacts. Pour les réponses produites à partir de documents (dossier, pièces
  jointes), chaque référence — y compris les lois, décrets et arrêtés — est aussi marquée
  🔎 *hors sources* quand elle ne figure dans rien de ce qui a été fourni au modèle
  (documents, résultats d'outils MCP, extraits RAG) : elle vient alors de sa mémoire.
  Nouvelle route `POST /api/juridique/verifier-citations`.
- **Serveurs MCP officiels (Légifrance, Judilibre)** — comme dans Demeter, Juridix importe
  ces deux serveurs depuis la release GitHub du pack (SHA-256 vérifié ; les autres serveurs
  du pack sont ignorés) puis les lance automatiquement en STDIO comme sous-processus.
  Nouveau panneau « Serveurs officiels » dans Réglages > Serveurs MCP : importer / mettre
  à jour, état, activation. Les identifiants PISTE de Juridix sont transmis aux serveurs en
  mémoire, sans être écrits sur disque. Le panneau affiche le nombre réel d'outils (« 6 sur 62 »)
  et laisse choisir ce qui est exposé au LLM : *Essentiels* (6 + 3, ≈ 1 450 jetons par requête, par
  défaut), *Tous* (62 + 8, ≈ 7 600 jetons) ou une liste *personnalisée* ; le coût de chaque option
  est annoncé avant de choisir. Routes
  `/api/mcp/stdio/servers`, `/api/mcp/stdio/pack-update`, `/api/mcp/stdio/stop`.

- **Veille juridique** — enregistrez des critères (mots-clés, fonds Légifrance, nature, code,
  juridictions Judilibre) : Juridix les rejoue en tâche de fond (toutes les 6 h au minimum, 24 h
  par défaut) et signale les textes et décisions **nouveaux** par un badge 🔔. Au démarrage, la
  période où l'application était fermée est rattrapée. Écran de gestion et de résultats dans le
  panneau juridique, ouverture directe dans la visionneuse. L'API PISTE est ménagée (une
  exécution à la fois, appels espacés, arrêt au premier HTTP 429). Routes `/api/veilles`.
- **« Compléter via Légifrance / Judilibre »** — les analyses de dossier (synthèse, contradictoire,
  points clés) n'utilisent plus aucun outil par défaut ; une case, décochée à chaque session,
  autorise uniquement ces deux serveurs, avec une consigne qui impose de signaler les compléments.

### Corrigé

- **Routage des outils MCP** — le préfixe d'un serveur MCP pouvait capter les outils d'un
  autre serveur dont le nom commence par le même préfixe.

- **Chiffrement au repos** — la base SQLite locale (conversations) est désormais chiffrée
  via SQLCipher (`rusqlite`, feature `bundled-sqlcipher-vendored-openssl`) ; migration
  automatique et transparente d'une base en clair préexistante au premier démarrage
  (sauvegarde `.pre-encryption.bak` conservée).
- **Trousseau système pour les secrets** — `bearer` (LLM), `qdrant_api_key` et
  `piste_client_secret` ne sont plus stockés en clair dans le `localStorage` du frontend :
  ils passent par le trousseau du système d'exploitation (Keychain/Credential
  Manager/Secret Service) via de nouvelles commandes Tauri `store_secret`/`read_secret`/
  `delete_secret`. Migration automatique et silencieuse d'une installation antérieure.
  Repli sur `localStorage` (avec avertissement console) hors de l'app desktop Tauri.

- **Pré-filtre local Winnow (optionnel)** — un modèle de décision servi par Ollaya trie les passages des
  documents avant envoi au LLM (hors sujet, mentions sans contenu juridique). Échec ouvert, omissions
  signalées au LLM. Activation par case à cocher dans Réglages ⚙ → Juridique (préférence utilisateur,
  transmise par requête — pas une variable d'environnement). Voir `docs/WINNOW.md`.
- Pastille 🌾 (à côté de « Analyser le dossier avec l'IA ») signalant l'état du pré-filtre, et fenêtre
  dédiée affichant sa sortie complète (statistiques, passages écartés avec extrait/motif/probabilité).
- Routes `GET /api/juridique/winnow-status?enabled=…` et `POST /api/juridique/winnow-warmup`.
- Champ facultatif « Question ou point précis » dans l'analyse de dossier.

### Corrigé

- `contenuLlm` conserve désormais la structure en paragraphes (`htmlToPlainBlocks`) au lieu d'écraser tout
  le texte sur une ligne, ce qui collait par ailleurs des mots entre balises adjacentes.

### Sécurité

- **Serveurs MCP : autorisation globale, sans confirmation à chaque appel** — un serveur MCP
  externe n'est utilisé par le modèle qu'après une autorisation donnée une fois pour tous ses outils
  (fenêtre listant les outils à l'ajout du serveur, ou bannière non bloquante au premier usage),
  liée à une empreinte de ses outils : s'il les modifie, l'autorisation tombe et est à renouveler.
  Filtre appliqué côté backend à chaque message ; serveurs officiels `stdio:` fiables par
  construction ; badge d'état dans les réglages ; seuls les outils annoncés au modèle sont
  exécutables (suppression du repli sur le premier serveur de la liste pour un nom d'outil inventé).
  Autorisations incluses dans l'export RGPD et effacées par « Tout effacer ». Voir
  `docs/MCP_AUTORISATION.md`
- **Pack MCP : signature des releases** — `checksums.txt` est vérifié avec une signature
  minisign (`checksums.txt.minisig`) avant tout usage, via une clé publique compilée dans
  l'application (`JURIDIX_MCP_PACK_PUBKEY`) ; échec fermé (release non signée, signature invalide,
  autre clé, format legacy ou rejeu de la signature d'une autre release : rien n'est installé), refus
  de tout retour à une version antérieure à celle installée, panneau « Serveurs officiels » indiquant
  si la signature est exigée. Sans clé compilée, mode de transition (SHA-256 seul), signalé dans
  l'interface. Les surcharges `JURIDIX_MCP_PACK_REPO` / `JURIDIX_MCP_PACK_VERSION` ne sont plus
  honorées qu'en build de développement. Procédure et script : `docs/MCP_PACK_SIGNATURE.md`,
  `scripts/sign-mcp-pack-release.sh`
- **Secrets : plus de repli en clair** — si le trousseau système échoue, les identifiants ne
  sont plus écrits dans le `localStorage` du WebView : ils restent en mémoire pour la session, avec
  une bannière d'avertissement ; les résidus d'anciens replis sont migrés puis supprimés
- **Sauvegarde en clair de la migration SQLCipher** — créée en 0600, signalée au démarrage,
  supprimable (écrasement puis suppression) depuis les réglages, détruite par l'effacement RGPD
- **API HTTP locale protégée** — jeton aléatoire par lancement (commande
  Tauri `get_api_token`, en-tête `X-Juridix-Token`), contrôle de `Host`
  (anti DNS-rebinding) et d'`Origin`, CORS restreint aux origines de
  l'application (auparavant `Any`, sans authentification : n'importe quel
  site web pouvait lire/supprimer les conversations, réécrire les prompts
  ou exfiltrer le jeton Albert)
- **Jeton Albert** — `/api/users/me` et `/api/models` utilisent désormais
  l'endpoint enregistré par `init_credentials` et non plus celui fourni
  dans la requête
- **Images externes supprimées** — le proxy d'images (`/api/image-proxy`), le rendu
  des images Markdown et la résolution Wikimedia sont retirés : Juridix n'affiche
  plus aucune image et ne contacte plus aucun hôte web hors de ses services (une
  image Markdown produite par le LLM pouvait servir de canal d'exfiltration par
  injection de prompt). Le texte alternatif est affiché de façon inerte. Le jeton
  d'API n'est donc plus jamais accepté en paramètre d'URL
- **Aucune ressource tierce au chargement** — polices (Sora, DM Sans, JetBrains Mono,
  Inter, Ubuntu) et feuille de style KaTeX embarquées (`@fontsource/*`, `katex`)
  au lieu de Google Fonts / jsDelivr ; CSP resserrée (`style-src`, `font-src`,
  `img-src` et `connect-src` sans domaine externe)
- **Serveurs MCP HTTP distants** — badge « ⚠ distant », avertissement permanent et
  confirmation explicite avant l'ajout d'un serveur hors `localhost`
- **Rendu SVG** — sanitisation par DOM (liste blanche d'éléments, suppression
  des `on*`, `href` externes et `url()` externes) au lieu d'expressions
  régulières
- **Journaux** — suppression de la trace partielle de la clé Qdrant
- **Extraction DOCX** — borne de taille décompressée (bombe ZIP)
- **Routes sensibles** — l'installation/mise à jour du pack de serveurs MCP
  (`/api/mcp/stdio/*`, binaires ensuite exécutés), la veille et la
  vérification de citations exigent désormais le jeton d'API : un site web
  tiers ne peut plus les déclencher
- **Dépendances npm** — `npm audit fix` (nanoid, fast-uri, dompurify, mermaid,
  postcss, katex…) : plus aucune alerte « high » ; reste ECharts < 6.1.0
  (XSS d'infobulle, CVE-2026-45249), neutralisée par échappement des noms des
  séries `lines` en attendant la montée de version
- **Serveurs MCP HTTP** — schémas non HTTP et adresses link-local / métadonnées
  cloud refusés (le loopback et le réseau privé restent permis) ; les redirections
  ne sont plus suivies automatiquement mais à la main (3 sauts max), chaque saut
  étant re-validé, sans rétrogradation https → http
- **CI** — actions GitHub épinglées par SHA de commit
- **CI** — job `audit` (cargo audit + npm audit), exécuté aussi chaque semaine

---
## [1.1.3] — non publiée
## [1.1.2] — non publiée
## [1.1.1] — non publiée
## [1.1.0] — non publiée
## [1.0.5] — non publiée
---

## [1.0.0] — 2026-08-02

### Première version publique

Première publication de Juridix, assistant IA desktop spécialisé en droit
français (Tauri v2 · Rust · React), fonctionnant entièrement en local.

### Ajouté

- **Recherche juridique intégrée** — accès direct aux données officielles
  Légifrance et JUDILIBRE, avec panneau de recherche dédié, dossiers,
  favoris, annotations et visionneuse de documents
- **Résolution inline des citations d'article** — un lien de citation vers
  un article affiche désormais son texte directement sous le lien plutôt
  qu'uniquement au survol ou après navigation ; portée limitée aux liens
  vers un article (LEGIARTI/JORFARTI/KALIARTI/CNILARTICLE), plafonnée à 40
  liens résolus par document, récupérés par lots de 6 requêtes simultanées
- **Historique et modifications d'un article Légifrance** — la fiche
  article affiche les sections « Modifié par » (textes modificateurs, avec
  lien direct vers l'article précis quand disponible) et « Historique »
- **Navigation par citation entre fonds Légifrance** — la navigation par
  citation route par la forme de l'identifiant plutôt que par le filtre de
  recherche courant (CODE, LODA, JORF), pour éviter les échecs silencieux
  d'ouverture entre fonds différents
- **Backend Rust (Axum) intégré à Tauri v2** — serveur HTTP local, sans
  dépendance externe obligatoire (aucun Docker, aucun cloud requis pour le
  cœur de l'application)
- **Pipeline RAG complet via Qdrant** — ingestion, chunking, embeddings et
  reranking, avec authentification JWT granulaire par collection et URL
  Qdrant configurable par utilisateur
- **Support MCP (Model Context Protocol)** — transport HTTP Streamable,
  serveurs configurables avec alias et statut de connexion en temps réel
- **Génération de documents Word (`.docx`)** directement depuis le chat
- **Rendu de graphiques (ECharts), diagrammes (Mermaid), SVG natif et
  formules mathématiques (KaTeX)** dans les réponses
- **Mode Comparaison** — soumission simultanée d'une question à deux
  modèles, avec choix de la réponse préférée
- **Personnalisation de l'apparence** — choix de police et de taille avec
  aperçu en temps réel
- **Éditeur de prompts intégré** et persistance SQLite locale des
  conversations, sans service externe

---
