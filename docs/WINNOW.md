# Pré-filtre local « Winnow » (Ollaya)

Couche **optionnelle** entre l'extraction des textes (API PISTE : Légifrance / JUDILIBRE) et le LLM.
Un *modèle de décision* servi **sur la machine de l'utilisateur** par [Ollaya](https://ollaya.dev)
trie les passages du texte avant construction du prompt.

```
API PISTE ─► extraction (frontend) ─► /envoyer-chat ─► segmentation ─► Winnow (local)
                                                                          │
                                       LLM distant  ◄─ prompt allégé ◄────┘  politique prudente
```

> **Winnow ≠ l'algorithme de Littlestone.** Ici, [Winnow](https://ollaya.dev/library/winnow) est un
> modèle de décision (fine-tuning de Gemma 4, GGUF, Apache-2.0) : il ne génère pas de texte, il répond à
> des questions typées (choix, score) avec des probabilités calibrées, en une passe.

## Interface

Une pastille 🌾 apparaît à côté de « Analyser le dossier avec l'IA » (visible même filtre désactivé, pour
rappeler que la fonctionnalité existe) :

| Pastille | Signification |
|---|---|
| `🌾 Winnow désactivé` | case décochée dans les Réglages |
| `🌾 Winnow…` | vérification de l'état d'Ollaya en cours |
| `🌾 Winnow inactif` | activé mais Ollaya ou le modèle n'est pas disponible (survoler/cliquer pour le détail) |
| `🌾 Winnow prêt` | activé, Ollaya joignable, modèle installé — aucune analyse encore lancée |
| `🌾 Winnow non appliqué` | une analyse a eu lieu mais le filtre a échoué ouvert (raison affichée) |
| `🌾 Winnow · N/M écartés` | une analyse a filtré M passages, dont N écartés |

Cliquer dessus ouvre une fenêtre dédiée : état complet d'Ollaya (avant une analyse), ou, après une analyse,
le rapport intégral — statistiques et la liste de chaque passage écarté (extrait, motif, probabilité).

## Ce qui est décidé, pour chaque passage

| Question | Type | Posée quand | Effet |
|---|---|---|---|
| `relevance` — utilité pour la demande | score 0–3 | une demande (champ « Question ou point précis ») est fournie | écarté si P(niveau 0) ≥ `WINNOW_DROP_THRESHOLD` |
| `role` — règle / motivation / contexte / annexe / mentions | choix | toujours | écarté si « mentions » (en-têtes, signatures, formules) avec P ≥ `WINNOW_ROLE_THRESHOLD` |

Sans demande, le filtre se borne donc à retirer le bruit sans contenu juridique ; il ne juge jamais la
pertinence dans le vide.

## Garde-fous (propres au domaine juridique)

1. **Échec ouvert** — Ollaya éteint, modèle absent, délai dépassé, erreurs répétées : le texte part
   *intégralement*, comme avant. Un disjoncteur évite de payer un timeout à chaque requête.
2. **Transparence** — chaque omission est repérée par un marqueur `[… N passage(s) écarté(s) … ]` dans le
   prompt, le LLM est prévenu de ne pas conclure à l'absence d'une disposition, et l'UI liste tout ce qui a
   été retiré (extrait, motif, probabilité).
3. **Prudence** — seuils hauts (0,85) ; jamais écartés : le 1ᵉʳ passage de chaque document, les passages qui
   citent un article mentionné dans la demande (`L. 1234-5`…), les passages tronqués par Ollaya ; plancher de
   20 % des passages conservés par document (on restitue d'abord les écartements les moins sûrs).
4. **Aucun mot modifié** — les passages conservés sont repris tels quels ; un document où rien n'est écarté
   est renvoyé strictement inchangé.

## Installation

```bash
# Ollaya ≥ 0.7.0 (exécute les GGUF via llama.cpp) — https://ollaya.dev/download
ollaya serve                 # écoute sur 127.0.0.1:11435
ollaya pull winnow:e4b       # 8,0 Go ; winnow:12b = 12,7 Go, un peu plus précis selon l'auteur
```

Puis, dans Juridix : **Réglages ⚙ → Juridique — API PISTE → 🌾 Pré-filtre local Winnow**, cocher la case.

Contrairement aux autres réglages Winnow ci-dessous (URL du serveur, modèle, seuils), qui décrivent une
installation locale, **l'activation n'est pas une variable d'environnement** : c'est une préférence
utilisateur, stockée comme le reste des Réglages, et envoyée à chaque requête d'analyse
(`EnvoyerChatRequest.winnow_enabled`). Deux instances de Juridix sur le même poste peuvent donc faire des
choix différents, et le changement prend effet immédiatement, sans redémarrage.

Matériel : `winnow:e4b` demande ≈ 9 Go de VRAM ; sur CPU il faut ≈ 5 s **par passage** (mesure de la doc
Ollaya) — inutilisable pour un dossier. Au premier usage le modèle n'est pas en mémoire : Juridix le charge
en arrière-plan et **ce premier envoi part sans filtre** ; `POST /api/juridique/winnow-warmup` évite cela.
`GET /api/juridique/winnow-status?enabled=true` renvoie l'état (joignable, modèle installé/chargé,
périphérique) — la pastille 🌾 à côté de « Analyser le dossier avec l'IA » l'affiche, et un clic dessus
ouvre une fenêtre dédiée avec le détail complet (y compris, après une analyse, chaque passage écarté).

## Réglages (`.env`)

Ces variables portent sur *où* et *comment* Ollaya tourne (détails d'installation locale) ; l'activation
elle-même (*si* le filtre tourne) est une case à cocher dans les Réglages, pas une variable d'environnement.

| Variable | Défaut | Rôle |
|---|---|---|
| `OLLAYA_URL` / `OLLAYA_API_KEY` | `http://127.0.0.1:11435` / — | serveur Ollaya |
| `WINNOW_MODEL` | `winnow:e4b` | modèle de décision |
| `WINNOW_DROP_THRESHOLD` | `0.85` | seuil « hors sujet » |
| `WINNOW_ROLE_THRESHOLD` | `0.85` | seuil « sans contenu juridique » |
| `WINNOW_MIN_KEEP_RATIO` | `0.2` | plancher de passages conservés par document |
| `WINNOW_TIME_BUDGET_SECS` | `30` | budget global ; au-delà, le reste est conservé |
| `WINNOW_MAX_PASSAGES` | `400` | plafond de passages scorés par requête |
| `WINNOW_MIN_CHARS` | `8000` | en dessous, aucun appel (gain nul) |
| `WINNOW_MAX_PASSAGE_CHARS` / `WINNOW_MIN_PASSAGE_CHARS` | `5000` / `300` | taille des passages (contexte Winnow : 8192 tokens) |
| `WINNOW_KEEP_ALIVE` | `30m` | maintien du modèle en mémoire |
| `WINNOW_REQUEST_TIMEOUT_SECS` / `WINNOW_LOAD_TIMEOUT_SECS` | `20` / `90` | délais |

## Limites connues — à lire avant d'activer en production

- **Qualité non mesurée sur du droit français.** Les benchmarks publiés pour Winnow portent sur des tâches
  de décision généralistes. Avant de s'y fier : constituer 50–100 passages annotés (garder / écarter) issus
  de vos dossiers, mesurer le rappel des passages utiles, puis ajuster les seuils. Le rappel prime sur la
  précision : un passage utile écarté coûte bien plus qu'un passage inutile conservé.
- **Périmètre actuel** : dossiers et documents envoyés via « Analyser le dossier » / `envoyer-chat`. Pas
  (encore) les chunks RAG (le reranker fait déjà ce travail) ni les résultats d'outils MCP (JSON : un
  découpage textuel risquerait d'en casser la structure).
- **Dossiers déjà stockés** : leur `contenu` a été aplati par l'ancien `stripHtml` ; le segmenteur sait
  retrouver les coupures sur les marqueurs usuels (`Article…`, `Attendu que…`, `Vu…`) mais un dossier
  ré-ajouté avec la version actuelle sera mieux découpé.
- **Modèles encodeurs plus légers** (`nli`, `gliclass`, `laya`) : mêmes formes de réponse, utilisables sur CPU,
  mais contextes courts (`laya` : 512 tokens → `WINNOW_MAX_PASSAGE_CHARS≈1200`). Non testés ici.

## Pistes suivantes

- `ollaya create -f Modelfile` pour figer les questions et **recalibrer** les températures sur vos données annotées.
- Étendre à la sortie des outils MCP Légifrance (après extraction du texte des champs JSON).
- Question supplémentaire « ce passage vise-t-il une version abrogée ? » à partir des métadonnées `etat`.
