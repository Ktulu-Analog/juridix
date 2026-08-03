# Juridix

> Assistant juridique souverain — application desktop cross-platform (Tauri v2 · Rust · React)

[![CI](https://github.com/Ktulu-Analog/juridix/actions/workflows/ci.yml/badge.svg)](https://github.com/Ktulu-Analog/juridix/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.77+-orange?logo=rust)](https://rustup.rs)

Juridix est un assistant IA desktop spécialisé en droit français. Il permet d'**interroger
directement les sources officielles** — **Légifrance** (codes, lois, décrets, JO, conventions
collectives…) et **JUDILIBRE** (jurisprudence judiciaire, administrative, constitutionnelle) —
puis de **faire traiter les documents consultés par une IA** : synthèse, analyse contradictoire,
extraction des points clés, réponses sourcées à jour.


## Aperçu

<p align="center">
  <img src="docs/juridix-1.png" alt="Juridix — capture d'écran 1" width="49%" />
  <img src="docs/juridix-2.png" alt="Juridix — capture d'écran 2" width="49%" />
</p>
<p align="center">
  <img src="docs/juridix-3.png" alt="Juridix — capture d'écran 3" width="49%" />
  <img src="docs/juridix-4.png" alt="Juridix — capture d'écran 4" width="49%" />
</p>



L'application est **souveraine par conception** :

- **Aucune donnée envoyée à un tiers non choisi par l'utilisateur.** Le cœur applicatif
  (recherche, RAG, persistance) tourne entièrement en local, sans compte ni service
  obligatoire — aucun Docker, aucun cloud requis.
- **Sources juridiques officielles** via l'**API PISTE**  (Légifrance +
  JUDILIBRE), en OAuth2 direct — pas d'intermédiaire commercial.
- **IA compatible Albert**, la plateforme de la DINUM (`https://ia.numerique.gouv.fr/outils-ia/albert-api/`),
  utilisable pour le chat, les embeddings et le reranking — ou tout autre endpoint
  compatible OpenAI (Mistral, OpenAI, Ollama en local…) au choix de l'utilisateur.
- **Aucun stockage tiers** : conversations en SQLite local, index vectoriel RAG dans un
  Qdrant local ou auto-hébergé — jamais de service SaaS imposé.

---

## Fonctionnalités

| Capacité | Détail |
|---|---|
| ⚖️ **Recherche juridique dédiée** | Panneau de recherche Légifrance + JUDILIBRE (codes, lois, décrets, JO, conventions collectives, jurisprudence judiciaire/administrative/constitutionnelle, CNIL…), dossiers, favoris et annotations |
| 💬 **Chat IA juridique** | Espace « Juris » avec accès outillé (MCP) aux 62 outils Légifrance et à JUDILIBRE pour des réponses sourcées et à jour |
| 📋 **Traitement IA des documents consultés** | Synthèse, analyse contradictoire et extraction des points clés d'un document ou d'un dossier constitué depuis la recherche juridique |
| 📚 **RAG (Qdrant)** | Ingestion PDF/DOCX (jurisprudence, doctrine, contrats…), recherche sémantique + reranking ; stockage vectoriel via **Qdrant** local ou distant |
| 🔌 **MCP** | Connexion à des serveurs MCP externes (HTTP Streamable 2025-03-26) |
| 🌐 **Recherche web** | Serveur MCP dédié (disponible sur mon GitHub ou utilisez votre propre serveur MCP Web)— activé à la demande via le bouton « Web » ; alias et statut visibles en info-bulle |
| 📄 **Génération Word** | Export `.docx` depuis le chat en un clic |
| 📊 **Graphiques ECharts** | Blocs ` ```echarts ` rendus interactivement |
| 🖼️ **Graphiques SVG natifs** | Blocs ` ```svg ` rendus inline avec zoom, copie et téléchargement |
| 📐 **Diagrammes Mermaid** | Flowcharts, séquences, Gantt… |
| 💾 **Persistance locale** | Conversations en SQLite — aucun compte requis |
| 🖥️ **Cross-platform** | Linux · macOS · Windows |

---

## Architecture

```
juridix/
├── frontend/                   ← React 18 + TypeScript + Vite
│   └── src/
│       ├── App.tsx             ← UI principale & gestion de l'état
│       ├── DialogContext.tsx   ← Contexte des dialogues
│       ├── components/
│       │   ├── MarkdownComponents.tsx   ← Rendu Markdown + ECharts + Mermaid + SVG
│       │   ├── IngestionModal.tsx       ← Interface RAG (Qdrant)
│       │   ├── ArtifactsPanel.tsx       ← Panneau Word/artefacts
│       │   ├── PromptsEditorModal.tsx   ← Éditeur d'espaces & prompts
│       │   ├── JuridiquePanel.tsx       ← Panneau de recherche Légifrance/JUDILIBRE
│       │   ├── juridique/               ← Hooks & extraction de contenu juridique
│       │   └── Message.tsx             ← Rendu d'un message
│       ├── hooks/              ← Hooks React (streaming, MCP…)
│       └── utils/              ← docx, text, mermaid-sanitizer, artifacts
│
└── src-tauri/                  ← Backend Rust + config Tauri
    └── src/
        ├── main.rs             ← Point d'entrée binaire
        ├── lib.rs              ← Setup Tauri + démarrage serveur HTTP
        ├── api.rs              ← Endpoints Axum (chat, RAG, MCP, search…)
        ├── config.rs           ← Configuration & variables d'environnement
        ├── db.rs               ← SQLite via tokio-rusqlite
        ├── models.rs           ← Types Serde (Chat, Conversation, Chunk…)
        ├── rag.rs              ← Pipeline RAG Qdrant (retrieve → rerank → inject)
        ├── mcp.rs              ← Client MCP HTTP Streamable
        ├── extract.rs          ← Extraction texte PDF (lopdf) et DOCX (zip+XML)
        ├── spaces.rs           ← Lecture/écriture prompts.yml
        └── juridique/          ← Module Recherche juridique (API PISTE)
            ├── client.rs       ← Client OAuth2 PISTE (token, refresh auto)
            ├── legifrance.rs   ← Wrappers API Légifrance
            ├── judilibre.rs    ← Wrappers API JUDILIBRE (jurisprudence)
            └── routes.rs       ← Handlers Axum du module juridique
```

### Stack technique

| Couche | Technologie |
|---|---|
| Desktop shell | [Tauri v2](https://tauri.app) |
| Backend | Rust · [Axum](https://github.com/tokio-rs/axum) · [tokio](https://tokio.rs) |
| Base de données | SQLite via [tokio-rusqlite](https://github.com/programatik29/tokio-rusqlite) |
| Stockage vectoriel | [Qdrant](https://qdrant.tech) local ou distant |
| HTTP client | [reqwest](https://github.com/seanmonstar/reqwest) (rustls, HTTP/2) |
| Frontend | React 18 · TypeScript · Vite |
| Rendu | react-markdown · KaTeX · Mermaid · ECharts · SVG natif |
| Documents | [docx](https://www.npmjs.com/package/docx) (génération Word) |
| Données juridiques | API PISTE (Légifrance + JUDILIBRE) via [piste.gouv.fr](https://piste.gouv.fr)|
| IA (chat, embeddings, reranking) | Toute API compatible OpenAI — par défaut [Albert](https://ia.numerique.gouv.fr/outils-ia/albert-api/) (DINUM) |

---

## Prérequis

### Rust ≥ 1.77
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### Node.js ≥ 18
```bash
nvm install 20 && nvm use 20
```

### Tauri CLI v2
```bash
cargo install tauri-cli --version "^2"
# ou via npm :
npm install -g @tauri-apps/cli@^2
```

### Qdrant

Qdrant est le moteur de recherche vectorielle utilisé par Juridix pour le RAG. Installez-le localement :

```bash
# Docker (recommandé)
docker run -d -p 6333:6333 -p 6334:6334 \
  -v $(pwd)/qdrant_storage:/qdrant/storage \
  qdrant/qdrant

# ou binaire natif — voir https://qdrant.tech/documentation/guides/installation/
```

L'URL par défaut est `http://localhost:6333`. Vous pouvez pointer vers une instance distante via le champ **URL Qdrant** dans les paramètres ou la variable d'environnement `QDRANT_URL`.

### Dépendances système

**Linux (Debian/Ubuntu)**
```bash
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev \
  patchelf build-essential
```

**macOS** — `xcode-select --install`

**Windows** — aucune dépendance supplémentaire (WebView2 inclus avec Windows 10/11).

---

## Démarrage rapide

```bash
git clone https://github.com/<votre-org>/juridix.git
cd juridix

cd frontend && npm install && cd ..
cargo tauri dev
```

L'interface s'ouvre automatiquement.
Le serveur Rust écoute sur `http://localhost:51873`.

### Build production

```bash
cd frontend && npm install && npm run build && cd ..
cargo tauri build
```

Les installateurs sont générés dans `src-tauri/target/release/bundle/` :
- Linux : `.deb` + `.AppImage`
- macOS : `.dmg`
- Windows : `.msi` + `.exe`

---

## Configuration

### Connexion au LLM

Au premier lancement, ouvrez **Paramètres** (⌘, / Ctrl+,) et renseignez :

- **Endpoint** — URL de l'API compatible OpenAI  
  ex. `https://albert.api.etalab.gouv.fr/v1`
- **Clé API** — votre token Bearer
- **Modèle** — identifiant du modèle  
  ex. `meta-llama/Llama-3.1-8B-Instruct`

### Accès Légifrance / JUDILIBRE (API PISTE)

Le panneau de recherche juridique et l'espace de chat « Juris » s'appuient sur l'API
officielle **PISTE** (Légifrance + JUDILIBRE). Créez un compte sur
[piste.gouv.fr](https://piste.gouv.fr), générez des credentials OAuth2, puis renseignez-les
dans **Paramètres** ou via les variables d'environnement `PISTE_CLIENT_ID` /
`PISTE_CLIENT_SECRET`. Sans ces credentials, le module juridique reste désactivé.

### Connexion Qdrant

Dans les **Paramètres**, section RAG :

- **Clé API Qdrant** — laissez vide si votre instance n'est pas sécurisée ; entrez une clé admin brute ou un JWT pour les instances protégées
- **URL Qdrant** — laissez vide pour utiliser `http://localhost:6333` (défaut), ou saisissez l'URL de votre instance distante

Si votre clé Qdrant est un **JWT** (commence par `eyJ`), vous pouvez également configurer des **tokens par collection** pour un accès granulaire en lecture/écriture.

### Variables d'environnement

| Variable | Défaut | Description |
|---|---|---|
| `PISTE_CLIENT_ID` | _(vide)_ | Client ID OAuth2 PISTE (Légifrance/JUDILIBRE) — laisser vide pour désactiver le module juridique |
| `PISTE_CLIENT_SECRET` | _(vide)_ | Client secret OAuth2 PISTE |
| `QDRANT_URL` | `http://localhost:6333` | URL de l'instance Qdrant |
| `QDRANT_API_KEY` | _(vide)_ | Clé d'accès Qdrant (admin ou JWT) |
| `EMBEDDING_MODEL` | `BAAI/bge-m3` | Modèle d'embedding Albert |
| `EMBEDDING_DIM` | `1024` | Dimension des vecteurs |
| `RERANK_MODEL` | `BAAI/bge-reranker-v2-m3` | Modèle de reranking |
| `RAG_TOP_K` | `20` | Chunks récupérés avant reranking |
| `RAG_TOP_RERANK` | `5` | Chunks conservés après reranking |
| `RAG_MIN_SCORE` | `0.15` | Score minimum (filtre qualité) |
| `RUST_LOG` | `juridix=info` | Niveau de log (`debug`, `info`, `warn`) |

### Espaces & Prompts

Les espaces sont configurés dans `src-tauri/prompts.yml` (embarqué dans le binaire, copié au premier lancement dans le répertoire de données utilisateur). L'éditeur intégré (**Espaces** → ⌘E / Ctrl+E) permet de les modifier sans toucher au YAML. Par défaut, Juridix embarque deux espaces : **Juris** (chat juridique outillé) et **Illustration** (schémas SVG).

---

## Données persistées

| OS | Chemin |
|---|---|
| Linux | `~/.local/share/Juridix/` |
| macOS | `~/Library/Application Support/Juridix/` |
| Windows | `%APPDATA%\Juridix\` |

- `conversations.db` — SQLite des conversations
- `prompts.yml` — configuration des espaces

---

## Recherche juridique dédiée

Le panneau **Recherche juridique** (accessible en vue par défaut) donne un accès direct,
hors chat, aux fonds Légifrance et JUDILIBRE :

- Recherche par fond (codes, JORF, LODA, jurisprudence judiciaire/administrative/
  constitutionnelle, conventions collectives, circulaires, délibérations CNIL…)
- Consultation d'articles avec résolution inline des citations et de l'historique des
  modifications
- Constitution de **dossiers** de documents, **favoris** et **annotations** locales
- Export et synthèse IA d'un document ou d'un dossier complet

## RAG — Ingestion de documents

1. Lancez **Qdrant** (`http://localhost:6333` par défaut)
2. Ouvrez **Ingestion RAG** (⌘R / Ctrl+R)
3. Créez ou sélectionnez une collection Qdrant
4. Déposez vos fichiers PDF, DOCX, TXT ou Markdown (jurisprudence, doctrine, contrats…)
5. Activez le RAG dans une conversation via le bouton 📚

Pipeline : extraction texte → chunking → embeddings (Albert) → stockage Qdrant → reranking (BGE) → injection contexte.

> **Note** : les valeurs de chunk sont exprimées en **caractères** (et non en tokens).  
> Les valeurs par défaut sont `chunk_size = 2048` et `chunk_overlap = 0`.

Un indicateur en temps réel (vert/rouge) dans la modale RAG signale l'état de la connexion Qdrant.

---

## Rendu graphique

Juridix supporte plusieurs formats de visualisation directement dans le chat :

- **ECharts** — blocs ` ```echarts ` avec JSON de configuration
- **SVG natif** — blocs ` ```svg ` rendus inline avec barre d'outils (copie presse-papier, téléchargement `.svg`)
- **Mermaid** — flowcharts, diagrammes de séquence, Gantt, etc.
- **KaTeX** — formules mathématiques inline et bloc

---

## MCP — Outils externes

Juridix supporte le [Model Context Protocol](https://modelcontextprotocol.io) en transport **HTTP Streamable** (spec 2025-03-26), compatible avec `mcp.data.gouv.fr` et tout serveur FastMCP stateless. C'est notamment via un serveur MCP que l'espace de chat « Juris » accède aux outils Légifrance (62 outils) et JUDILIBRE.

Ajoutez des serveurs MCP dans les paramètres. Chaque serveur peut recevoir un **alias** affiché dans l'interface. Les outils disponibles sont découverts automatiquement et les statuts de connexion sont visibles en info-bulle au survol des boutons MCP et Web.

Le bouton **Web** active un serveur MCP dédié à la recherche web (configurable séparément) ; il n'est inclus dans les appels que lorsqu'il est allumé.

---

## Souveraineté par conception

Juridix est pensé pour un usage juridique professionnel sans dépendance à une plateforme
étrangère ni à un service tiers non maîtrisé :

- **Exécution locale** — le backend Rust/Axum tourne dans le processus Tauri sur le poste
  de l'utilisateur ; aucune donnée ne transite par un serveur Juridix (il n'y en a pas).
- **Sources officielles françaises** — Légifrance et JUDILIBRE sont interrogés en direct
  via l'API **PISTE** de la DINUM, avec les identifiants OAuth2 propres à l'utilisateur.
- **IA souveraine par défaut** — le endpoint LLM proposé par défaut est **Albert**, l'API
  de la DINUM/Etalab ; tout autre endpoint compatible OpenAI (auto-hébergé via Ollama,
  ou tiers au choix) reste utilisable.
- **Aucun stockage tiers** — conversations et annotations en SQLite local ; index vectoriel
  RAG dans une instance Qdrant locale ou auto-hébergée par l'utilisateur. Aucune télémétrie,
  aucun compte, aucune synchronisation vers un service cloud imposé.

---

## Contribution

Les contributions sont les bienvenues ! Consultez [CONTRIBUTING.md](CONTRIBUTING.md) pour les détails.

```bash
# Fork → branche
git checkout -b feat/ma-fonctionnalite

# Vérifications avant PR
cargo fmt && cargo clippy
cd frontend && npm run typecheck
```

---

## Licence

[AGPL-3.0](LICENSE) — © 2026 **Pierre Couget** — [ktulu.analog@gmail.com](mailto:ktulu.analog@gmail.com)
