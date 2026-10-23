# Changelog

Toutes les modifications notables de ce projet sont documentées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/),
et ce projet adhère au [Versioning Sémantique](https://semver.org/lang/fr/).

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
