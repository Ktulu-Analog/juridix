# Autorisation des serveurs MCP externes

Un serveur MCP donne au modèle des **outils** qu'il peut appeler. Juridix n'expose donc les outils d'un
serveur externe au modèle qu'après une **autorisation de l'utilisateur, donnée une fois pour tous les outils
du serveur** — jamais une confirmation à chaque appel.

## Qui est concerné

| Serveur | Autorisation |
|---|---|
| Serveurs officiels `stdio:` (Légifrance, Judilibre) | Aucune : fiables par construction |
| Serveur HTTP sur cette machine (`localhost`, `127.0.0.1`, `::1`) | Oui |
| Serveur HTTP distant | Oui, avec badge « ⚠ distant » et confirmation explicite à l'ajout |

## Fonctionnement

1. À l'ajout d'un serveur, une fenêtre liste ses outils (nom et début de description). L'utilisateur autorise
   ou non. Pour un serveur déjà configuré, une bannière non bloquante s'affiche au premier message.
2. L'autorisation est enregistrée avec une **empreinte SHA-256** des outils : pour chacun, son nom, sa
   description, le schéma de ses paramètres et ses annotations (`readOnlyHint`…), indépendamment de
   l'ordre. L'utilisateur autorise exactement ce qu'il a vu (l'empreinte affichée est renvoyée telle quelle).
3. À **chaque message**, le backend relit la liste des outils du serveur et compare l'empreinte :

| Statut | Signification | Effet |
|---|---|---|
| `granted` | Autorisé, outils inchangés | Outils annoncés au modèle et exécutables |
| `new` | Jamais autorisé | Outils écartés, bannière « à autoriser » |
| `changed` | Autorisé, mais les outils ont changé | Outils écartés ; l'autorisation doit être renouvelée |
| `unavailable` | Serveur injoignable | Rien d'exposé, rien à autoriser |

Un serveur sans aucun outil est considéré comme autorisé (rien à autoriser).

## Garanties

- **Seuls les outils annoncés au modèle sont exécutables** : un nom d'outil inventé par le modèle n'est pas
  exécuté, et il n'y a pas de repli sur le premier serveur de la liste.
- Un serveur qui modifie ses outils après autorisation (description, paramètres…) perd son autorisation :
  c'est le scénario d'un serveur qui change de comportement après avoir été approuvé.
- L'application de la règle se fait **côté backend** (`src-tauri/src/mcp_grants.rs`) ; l'interface
  (`frontend/src/utils/mcpGrants.ts`) ne fait qu'afficher les outils et transmettre l'accord.

## Données

Les autorisations sont stockées dans la table `mcp_grants` (serveur, empreinte, noms des outils, date). Elles
sont incluses dans l'export RGPD et effacées par « Tout effacer ».
