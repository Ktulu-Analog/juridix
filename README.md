# Juridix

> Assistant juridique intelligent — application desktop cross-platform (Tauri v2 · Rust · React)

[![CI](https://github.com/Ktulu-Analog/juridix/actions/workflows/ci.yml/badge.svg)](https://github.com/Ktulu-Analog/juridix/actions/workflows/ci.yml)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.77+-orange?logo=rust)](https://rustup.rs)

Juridix est un assistant IA spécialisé en droit français, fonctionnant entièrement en
local sans service externe obligatoire — aucun Docker, aucun cloud requis pour le cœur
de l'application. Le backend **Rust/Axum** est embarqué dans l'application **Tauri v2**
et pilote un LLM via n'importe quelle API compatible OpenAI (Albert API, OpenAI, Mistral,
Ollama…), avec un accès direct aux données juridiques officielles **Légifrance** et
**JUDILIBRE**.

---

## Fonctionnalités

| Capacité | Détail |
|---|---|
| ⚖️ **Recherche juridique dédiée** | Panneau de recherche Légifrance + JUDILIBRE (codes, lois, décrets, JO, conventions collectives, jurisprudence judiciaire/administrative/constitutionnelle, CNIL…), dossiers, favoris et annotations |
| 💬 **Chat IA juridique** | Espace « Juris » avec accès outillé (MCP) aux 62 outils Légifrance et à JUDILIBRE pour des réponses sourcées et à jour |
| 🎨 **Illustration SVG** | Espace dédié à la génération de schémas et diagrammes vectoriels |
| 📚 **RAG (Qdrant)** | Ingestion PDF/DOCX (jurisprudence, doctrine, contrats…), recherche sémantique + reranking ; stockage vectoriel via **Qdrant** local ou distant |
| 🔌 **MCP** | Connexion à des serveurs MCP externes (HTTP Streamable 2025-03-26), notamment pour l'accès outillé à Légifrance/JUDILIBRE |
| 🌐 **Recherche web** | Serveur MCP dédié — activé à la demande via le bouton « Web » ; alias et statut visibles en info-bulle |
| 📄 **Génération Word** | Export `.docx` depuis le chat en un clic |
| 📊 **Graphiques ECharts** | Blocs ` ```echarts ` rendus interactivement |
| 🖼️ **Graphiques SVG natifs** | Blocs ` ```svg ` rendus inline avec zoom, copie et téléchargement |
| 📐 **Diagrammes Mermaid** | Flowcharts, séquences, Gantt… |
| ∑ **Formules KaTeX** | Rendu LaTeX inline (`$…$`) et bloc (`$$…$$`) |
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
        ├── api.rs              ← Endpoints Axum (chat, RAG, MCP, search…)
        ├── db.rs               ← SQLite via tokio-rusqlite
        ├── models.rs           ← Types Serde (Chat, Conversation, Chunk…)
        ├── rag.rs              ← Pipeline RAG Qdrant (retrieve → rerank → inject)
        ├── mcp.rs              ← Client MCP HTTP Streamable
        ├── extract.rs          ← Extraction texte PDF (lopdf) et DOCX (zip+XML)
        ├── spaces.rs           ← Lecture/écriture prompts.yml
        ├── juridique/          ← API Légifrance + JUDILIBRE (recherche, articles, décisions…)
        └── lib.rs              ← Setup Tauri + démarrage serveur HTTP
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
| Données juridiques | API PISTE (Légifrance + JUDILIBRE) via [piste.gouv.fr](https://piste.gouv.fr) |

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
