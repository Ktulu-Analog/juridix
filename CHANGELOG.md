# Changelog

Toutes les modifications notables de ce projet sont documentées dans ce fichier.

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.0.0/),
et ce projet adhère au [Versioning Sémantique](https://semver.org/lang/fr/).

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
