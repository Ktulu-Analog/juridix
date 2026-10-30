# Feuille de route — Juridix

Cette feuille de route décrit ce qu'il reste à construire pour que Juridix soit un outil **complet et fiable pour un juriste** (avocat, juriste d'entreprise, juriste de collectivité). Elle part de l'état de la version 1.1.5. **Dernier relevé de l'état d'avancement : 27 octobre 2026** (par lecture du code ; voir la légende ci-dessous).

> **Statut du document.** Rédigée à partir de la lecture du code et du README, sans retour d'utilisateurs juristes. Les priorités sont à **valider avec 5 à 10 utilisateurs** avant d'engager les phases 3 et suivantes. Les estimations (en semaines, pour un développeur à temps plein) sont des ordres de grandeur. Les numéros de version sont indicatifs.

> **Légende.** ✅ fait · 🟡 fait en partie (le reste est précisé) · ⬜ à faire. Les cases « manuelles » (essais sur installeur, release du pack) ne peuvent pas se constater dans le code : elles restent ⬜ jusqu'à confirmation.

---

## 1. Point de départ

### Acquis

- Recherche Légifrance et Judilibre via l'API PISTE, dossiers, favoris, annotations.
- Chat outillé (MCP) avec les serveurs officiels Légifrance et Judilibre, signature du pack vérifiée.
- Veille juridique en tâche de fond et synthèse de veille exportable en Word.
- Vérification des articles de codes cités par le modèle (en vigueur / abrogé / introuvable).
- RAG sur documents personnels (Qdrant), pré-filtre local optionnel (Winnow).
- Base chiffrée (SQLCipher), secrets dans le trousseau du système, export et effacement RGPD.
- Fonctionnement sans compte, avec n'importe quelle API compatible OpenAI.
- Vérification étendue des citations : décisions de la Cour de cassation (numéro de pourvoi), lois/décrets/ordonnances, IDCC, citations textuelles (`verify_ext.rs`).
- Recherche dans les codes par numéro d'article (« code civil, article 1140 ») et en plein texte par paliers (`code_query.rs`, `code_search.rs`) — *non publié, à valider sur PISTE réel*.
- Bouton « Copier » dans la bulle de sélection de la visionneuse — *non publié*.

### Écarts constatés

| Domaine | Écart |
|---|---|
| Fiabilité | ~~La vérification ne couvre que les articles de codes~~ — **corrigé** : décisions de la Cour de cassation, lois/décrets, IDCC et citations textuelles sont vérifiés. ECLI, décisions du Conseil d'État, des juridictions administratives et des cours d'appel sont couverts (reste à valider sur PISTE réel). |
| Droit dans le temps | ~~Pas de « version applicable à la date des faits »~~ — **traité pour les articles de codes** (réglage « Droit applicable au… », version applicable et modifications depuis, `temporal.rs`). Reste : lois/décrets hors codes et jurisprudence par rapport à la date des faits. |
| Longueur | Aucun contrôle de la taille du prompt par rapport à la fenêtre du modèle ; les gros dossiers ne sont pas traités par morceaux. |
| Sources | Conseil d'État et juridictions administratives, doctrine administrative, droit européen, accords d'entreprise : non couverts ou peu accessibles. |
| Production | L'export Word existe, mais pas de modèles de livrables, de notes de bas de page, de table des autorités ni de revue de contrat. |
| Calculs | Délais, prescriptions et indemnités ne sont pas calculés par du code déterministe. |
| Confidentialité | Pas de pseudonymisation avant envoi au LLM, pas de journal de ce qui est transmis. |
| Qualité | Aucun lanceur de tests côté frontend ; pas de jeu d'évaluation des réponses ; pas de version macOS. |
| Documentation | `docs/CHECKLIST_RELEASE.md` et `docs/MCP_PACK_SIGNATURE.md` sont cités mais absents. |

---

## 2. Principes directeurs

1. **La confiance avant les fonctions.** Un juriste qui rencontre une fausse référence cesse de se fier à l'outil. Toute nouvelle fonction d'IA est précédée de sa mesure (voir phase 1).
2. **Le calcul n'est pas du ressort du LLM.** Délais, indemnités et dates sont du code testé, avec la règle affichée.
3. **Ce qui sort de la machine doit être visible.** L'utilisateur doit pouvoir dire à tout moment ce qui est parti, et vers qui.
4. **Assistant, pas conseil.** L'outil aide à chercher et à rédiger ; la responsabilité reste celle du juriste, et l'interface le dit clairement.
5. **Un échec visible vaut mieux qu'une coupe silencieuse.** Troncatures, sources absentes et vérifications impossibles sont signalées.

---

## 3. Phases

### Phase 0 — Avant de publier la 1.1.5

Objectif : sortir une release propre.

| # | Tâche | Critère d'acceptation | Statut |
|---|---|---|---|
| 0.1 | Créer `docs/CHECKLIST_RELEASE.md` et `docs/MCP_PACK_SIGNATURE.md` (ou retirer les renvois) | Plus aucun lien mort dans le README, le CHANGELOG et le corps du brouillon de release | ✅ `docs/CHECKLIST_RELEASE.md`, `docs/MCP_PACK_SIGNATURE.md` et `docs/MCP_AUTORISATION.md` (aussi référencé, lui aussi absent) écrits d'après le code |
| 0.2 | Essai manuel sur installeur : import des serveurs officiels, lecture d'un DOCX, graphiques ECharts, formules KaTeX | Chaque parcours testé sur Linux et Windows, résultat noté dans la checklist | ⬜ Essai manuel, non constatable dans le code |
| 0.3 | Audit planifié (cron hebdomadaire, non bloquant pour les PR) et Dependabot ou Renovate | Une alerte nouvelle ouvre une issue sans casser les PR | ✅ Dependabot (cargo, npm ×2, GitHub Actions), audit non bloquant hors exécution planifiée, issue d'alerte ouverte par l'exécution hebdomadaire. À constater au premier lundi |
| 0.4 | Vérifier que la dernière release du pack Demeter contient `checksums.txt.minisig` | Installation du pack réussie depuis un installeur de release | ⬜ Procédure de contrôle documentée (`docs/MCP_PACK_SIGNATURE.md`) ; reste à l'exécuter sur la dernière release du pack |

**Effort : 2–3 jours.**

---

### Phase 1 — Fiabilité et confiance (v1.2)

Objectif : que chaque référence affichée soit vérifiable et que les limites soient visibles. **Effort : 6–8 semaines.**

**1.1 Vérification étendue des citations**
> **Statut : 🟡 Décisions de la Cour de cassation (pourvoi), décisions citées par ECLI, Conseil d'État, cours et tribunaux administratifs (numéro de requête), cours d'appel (numéro de RG), lois/décrets/ordonnances, IDCC et citations textuelles sont vérifiés avec des statuts distincts (`NonRetrouvee` ≠ `Introuvable`) et des tests (`verify_juris.rs`, `verify_ext.rs`). Champs et filtres alignés sur les spécifications PISTE (CETAT : `NUM_DEC`, pas `NUM_AFFAIRE` ni `ECLI` ; Judilibre : filtre `location`, `ecli` des résultats) et éprouvés contre un faux serveur qui les applique. Reste : valider sur PISTE réel (`cargo test --lib live_piste_juridictions -- --ignored`) — chaîne de recherche Judilibre par ECLI (champ ciblé ou texte intégral), forme réelle des titres CETAT, acceptation des textes CETAT par `/consult/juri` (documenté pour le fonds JURI) ; Conseil constitutionnel, CJUE, tribunaux judiciaires et de commerce (ECLI reconnus, non interrogés, ⚪)**
- Décisions : numéro de pourvoi ou ECLI → existence sur Judilibre, chambre, date, solution, publication.
- Lois, décrets, ordonnances : identifiant Légifrance, statut (en vigueur, abrogé).
- Conventions collectives : IDCC, existence et statut.
- Citations textuelles : le passage entre guillemets figure-t-il dans la source ? (correspondance exacte ou quasi exacte)
- *Critères* : chaque référence porte un statut visible (vérifiée, introuvable, texte non retrouvé) ; une référence inexistante n'est jamais présentée comme vérifiée ; jeu de tests avec références réelles, abrogées, inventées.

**1.2 Droit à date donnée**
> **Statut : 🟡 Fait pour les articles de codes : réglage « Droit applicable au… » (chat et recherche dans les codes), date transmise au modèle, version applicable à la date et modifications depuis calculées par le code (liste `articleVersions` de Légifrance) et affichées sur chaque citation. Reste : textes hors codes (lois, décrets), jurisprudence au regard de la date, volet « version à la date » dans la visionneuse. À valider sur PISTE réel (`cargo test --lib live_piste_droit_a_date -- --ignored`).**
- Sélecteur « droit applicable au… » dans la recherche et le chat ; le modèle reçoit la version consolidée à cette date.
- Signalement des textes modifiés ou abrogés depuis.
- *Critères* : pour un article modifié à une date connue, les deux versions sont correctement retrouvées selon la date choisie.

**1.3 Traitement de la jurisprudence**
> **Statut : 🟡 Fiche de décision (`GET /decision/:id/fiche`, `jurisprudence.rs`) : décision attaquée, décision suivante, rapprochements de jurisprudence (Judilibre) et liens typés (Légifrance), distinction fond / cassation, chacun avec sa provenance (base et champ) ; accessible depuis le badge d'une décision vérifiée (⛓). Les « cité par » et les revirements, que les API ne fournissent pas, sont listés comme indisponibles au lieu d'être inventés. Reste : valider sur PISTE réel (`cargo test --lib live_piste_fiche -- --ignored`)**
- Décisions postérieures, revirements, « cité par » quand l'API le permet, distinction fond / cassation.
- *Critères* : la fiche d'une décision indique les rapprochements disponibles et leur provenance.

**1.4 Gestion honnête de la longueur**
> **Statut : ⬜ Amorce seulement : la fenêtre de contexte des modèles est lue (`max_context_length`, `App.tsx`) mais n'est utilisée nulle part**
- Estimation du nombre de jetons avant envoi, avec avertissement si le prompt approche de la fenêtre du modèle (fenêtre lue dans la liste des modèles quand elle est fournie).
- Traitement par morceaux des gros dossiers (résumé de chaque document, puis synthèse), à la place des coupes silencieuses.
- *Critères* : un dossier de 30 documents longs produit une analyse sans erreur du fournisseur ; l'utilisateur voit ce qui a été résumé et ce qui a été transmis en entier.

**1.5 Réponses sourcées**
> **Statut : ⬜ Le prompt demande des réponses « sourcées », sans notes numérotées reliées au passage ni refus explicite**
- Notes numérotées reliées au passage exact de la source, ouvrables en un clic.
- Refus explicite (« aucune source trouvée ») plutôt que complément de mémoire, lorsque la question porte sur un texte ou une décision précise.

**1.6 Jeu d'évaluation**
> **Statut : ⬜**
- 50 à 100 questions avec réponses et références attendues, construites avec des juristes.
- Indicateurs : taux de références inexistantes, taux de références exactes, taux d'abstention justifiée.
- Exécution en CI à chaque changement de prompt, de modèle ou de logique de récupération.
- *Critères* : un seuil de non-régression bloquant la fusion.

**1.7 Tests de contrat avec PISTE**
> **Statut : ⬜ Les tests existants (`verify.rs`, `verify_ext.rs`, `code_search.rs`) s'appuient sur des réponses simulées à la main, non sur des enregistrements réels**
- Réponses réelles enregistrées (Légifrance et Judilibre) rejouées dans les tests ; échec si le code ne les comprend plus.
- *Origine* : la facette « Conseil d'État » de Judilibre produisait une erreur 400 qu'un tel test aurait détectée.

---

### Phase 2 — Couverture des sources (v1.3)

Objectif : trouver, au même endroit, ce qu'un juriste cherche vraiment. **Effort : 4–6 semaines.**

| # | Source | Détail | Statut |
|---|---|---|---|
| 2.1 | **Juridictions administratives** | Conseil d'État (fond `CETAT`), cours administratives d'appel et tribunaux administratifs (fond `JADE`), entrée de recherche claire distincte de Judilibre | 🟡 Fond `CETAT` (Conseil d'État) présent ; `JADE` (CAA, TA) absent |
| 2.2 | **Facettes issues de l'API** | Lire les valeurs depuis la taxonomie (`/taxonomy` pour Judilibre) au lieu de les coder en dur | 🟡 Taxonomies Judilibre lues via l'API (chambre, formation…) ; facettes Légifrance codées en dur |
| 2.3 | **Doctrine administrative** | BOFiP (fiscalité), BOSS (sécurité sociale), circulaires, réponses ministérielles | 🟡 Circulaires (`CIRC`) présentes ; BOFiP et BOSS absents |
| 2.4 | **Droit européen** | CJUE (EUR-Lex), CEDH (HUDOC) | ⬜ |
| 2.5 | **Accords et conventions** | Accords d'entreprise (`ACCO`), conventions collectives complètes (`KALI`) | 🟡 Fonds `KALI` et `ACCO` disponibles en recherche ; présence dans la veille et le chat non vérifiée |
| 2.6 | **Citation collée** | Coller « Cass. soc., 12 mars 2024, n° 22-12.345 » ouvre la décision | ⬜ Non constaté (des citations cliquables existent dans les réponses ; pas de collage libre d'une référence) |
| 2.7 | **Doctrine et modèles personnels** | Ingestion RAG enrichie : métadonnées (auteur, date, type), citations de doctrine dans les réponses | ⬜ Pas de métadonnées auteur/date/type dans l'ingestion RAG |

*Critères communs* : chaque source apparaît dans la recherche, la veille et le chat, avec le même format de référence et les mêmes statuts de vérification que la phase 1. Les conditions d'accès et de réutilisation (licences, quotas PISTE) sont documentées pour chaque source.

---

### Phase 3 — Travail quotidien et production de documents (v1.4)

Objectif : produire des livrables utilisables sans retouche lourde. **Effort : 8–10 semaines.**

**3.1 Dossiers enrichis**
> **Statut : 🟡 Dossiers, pièces issues des recherches et analyses (synthèse, contradictoire, points clés) existent. Reste : métadonnées (client, parties, échéances), import de pièces, chronologie**
- Métadonnées : client, nature, juridiction, parties, échéances.
- Pièces importées (PDF, DOCX, e-mails), chronologie des faits.

**3.2 Modèles de livrables**
> **Statut : ⬜**
- Consultation, note, mémo, courrier de mise en demeure, conclusions.
- Structure de raisonnement choisissable (majeure, mineure, conclusion).
- Modèles propres au cabinet, enregistrés et partageables par export.

**3.3 Export Word professionnel**
> **Statut : 🟡 L'export Word existe (synthèse de veille, messages). Reste : styles du cabinet, notes de bas de page, sommaire, table des autorités**
- Styles du cabinet, notes de bas de page, sommaire, **table des autorités citées**, conventions de citation françaises.
- *Critères* : un livrable exporté s'ouvre dans Word sans mise en forme à reprendre ; les références de la table des autorités correspondent aux notes.

**3.4 Revue de contrat**
> **Statut : ⬜**
- Extraction de clauses, comparaison à une grille de contrôle du cabinet.
- Propositions de modification en **suivi des modifications** Word.
- Comparaison de deux versions d'un document.

**3.5 Calculs déterministes**
> **Statut : ⬜**
- Délais de procédure et de recours, prescriptions (jours fériés, prorogations, point de départ), préavis, indemnités légales de licenciement.
- La règle appliquée et ses sources sont affichées ; chaque calcul est couvert par des tests issus de cas publiés.
- *Critères* : aucun résultat chiffré n'est produit par le modèle de langage seul.

**3.6 Pièces difficiles**
> **Statut : ⬜ `extract.rs` n'accepte que pdf, docx, doc, txt, md, csv et indique lui-même qu'il n'y a pas d'OCR (« Utilisez un outil d'OCR avant l'ingestion »)**
- PDF scannés (OCR), e-mails `.eml` et `.msg`, images. *À vérifier : l'état actuel de l'OCR dans `extract.rs`.*

**3.7 Recherche et suivi**
> **Statut : 🟡 Historique des recherches présent. Reste : requêtes sauvegardées, échéancier avec rappels**
- Requêtes sauvegardées, filtres par dates, historique, échéancier avec rappels liés aux dossiers.

---

### Phase 4 — Confidentialité démontrable (v1.5)

Objectif : permettre d'invoquer le secret professionnel et la conformité RGPD avec l'outil. **Effort : 6–8 semaines.**

**4.1 Pseudonymisation avant envoi au LLM**
> **Statut : ⬜**
- Détection et remplacement : noms, adresses, IBAN, numéros de sécurité sociale, immatriculations, dates de naissance.
- Réinjection des valeurs dans la réponse, localement.
- *Critères* : jeu de tests de documents annotés ; taux de détection mesuré et publié ; avertissement clair sur ce que la détection ne garantit pas.

**4.2 Panneau « où vont mes données »**
> **Statut : ⬜ Une fenêtre de confidentialité existe (`DataPrivacyModal`), non détaillée par requête**
- Pour chaque requête : ce qui part vers le LLM, vers PISTE (les termes de recherche aussi), vers chaque serveur MCP, vers Qdrant.

**4.3 Journal d'audit local**
> **Statut : ⬜**
- Historique horodaté des envois, exportable, consultable par l'utilisateur.

**4.4 Mode local de bout en bout**
> **Statut : 🟡 Fonctionne avec toute API compatible OpenAI. Reste : documentation, tests et réglage « dossier sensible »**
- Documentation et tests avec un LLM local (Ollama, vLLM) : aucun accès web, aucun serveur externe, pour les dossiers sensibles.
- Réglage « dossier sensible » qui désactive automatiquement les outils externes.

**4.5 Conservation et sauvegarde**
> **Statut : ⬜ Export et effacement RGPD présents ; pas de suppression automatique ni de sauvegarde/restauration**
- Suppression automatique des conversations après une durée choisie.
- Sauvegarde chiffrée et restauration testées.

---

### Phase 5 — Distribution et robustesse (en continu, dès la v1.2)

| # | Tâche | Détail | Statut |
|---|---|---|---|
| 5.1 | **macOS** | Build, signature et notarisation ; beaucoup de juristes sont sur Mac | ⬜ Matrice de release = Linux + Windows |
| 5.2 | **Signature et mise à jour** | Signature de code Windows, paquets Linux, mise à jour automatique signée | ⬜ Pas de mise à jour automatique ni de signature de code configurées |
| 5.3 | **Tests frontend** | Ajouter Vitest (aucun lanceur n'est configuré) ; tests de bout en bout sur recherche, dossier, veille, export | ⬜ Aucun lanceur ; seul `tsc --noEmit` côté frontend (`cargo test` tourne bien en CI côté Rust) |
| 5.4 | **Première configuration guidée** | Assistant avec boutons « tester la connexion » pour le LLM, PISTE et Qdrant ; les identifiants PISTE sont le principal obstacle à l'adoption | ⬜ |
| 5.5 | **Accessibilité** | Conformité RGAA, utile pour le secteur public | ⬜ Pas d'audit RGAA |
| 5.6 | **Conformité de la chaîne** | SBOM, `cargo-deny`, politique de mise à jour des dépendances | 🟡 `cargo audit` et `npm audit` en CI ; pas de SBOM ni de `cargo-deny` |
| 5.7 | **Guide utilisateur** | Guide pour juristes, sans jargon technique, avec cas pratiques | ⬜ README et `docs/WINNOW.md` seulement |
| 5.8 | **Performance** | Dossiers de plusieurs centaines de documents, listes de résultats longues | ⬜ Non évalué |

---

### Plus tard (v2) — Usage collectif

Dossiers et prompts partagés, rôles, authentification centralisée, hébergement serveur. C'est un changement d'architecture (aujourd'hui, application de bureau mono-utilisateur) : à n'engager qu'après avoir confirmé le besoin auprès de plusieurs structures.

---

## 4. Séquencement et dépendances

```
Phase 0 ──► Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 4
                │            │
                └── 1.6 (évaluation) conditionne les nouvelles fonctions d'IA
Phase 5 en parallèle dès Phase 1 (macOS, tests, première configuration)
```

- La **phase 1.6** (jeu d'évaluation) est un préalable aux phases 3 et 4 : sans mesure, impossible de savoir si une nouvelle fonction améliore ou dégrade la fiabilité.
- La **phase 1.1** (vérification étendue) alimente la table des autorités de la phase 3.3.
- La **phase 2.2** (facettes issues de l'API) se fait avant l'ajout de nouvelles sources.

## 5. Priorités si les ressources manquent

1. Vérification étendue des citations et des citations textuelles (1.1).
2. Gestion honnête de la longueur avec traitement par morceaux (1.4).
3. Conseil d'État et juridictions administratives (2.1).
4. Export Word professionnel avec table des autorités (3.3).
5. Pseudonymisation avant envoi (4.1).

## 6. Indicateurs de succès

| Indicateur | Cible indicative |
|---|---|
| Références inexistantes dans les réponses (jeu d'évaluation) | Proche de zéro, jamais en hausse d'une version à l'autre |
| Citations affichées avec un statut de vérification | 100 % |
| Sources principales accessibles depuis une seule interface | Judiciaire, administratif, Conseil constitutionnel, européen, doctrine administrative |
| Livrable exporté utilisable sans reprise de mise en forme | À mesurer sur un échantillon de juristes testeurs |
| Temps pour une première configuration réussie | Moins de 10 minutes avec l'assistant |
| Échecs d'installation du pack MCP signé | Aucun sur les installeurs de release |

## 7. Risques

| Risque | Parade |
|---|---|
| Fausse assurance : l'utilisateur croit tout vérifié | Statuts de vérification explicites, refus d'affirmer sans source, rappel de la responsabilité du juriste |
| Évolution des API PISTE | Tests de contrat (1.7), facettes lues depuis l'API (2.2) |
| Pseudonymisation imparfaite | Mesure publiée, avertissement, mode local pour les dossiers sensibles |
| Dérive des modèles et des prompts | Jeu d'évaluation en CI (1.6) |
| Dépendances vulnérables | Audit planifié et mises à jour automatisées (0.3, 5.6) |
| Réglementation (IA, secret professionnel) | Vérification juridique de la conformité selon l'usage : interne ou commercialisé |

## 8. Hors périmètre (pour l'instant)

- Rédaction autonome d'actes sans relecture ni validation par un juriste.
- Conseil juridique à destination du grand public.
- Prédiction de l'issue d'un litige (justice prédictive).

---

## 9. Prochaines étapes proposées (relevé du 26 octobre 2026)

### Bilan

| Phase | Fait | En partie | À faire |
|---|---|---|---|
| 0 — Release 1.1.5 | 2 (0.1, 0.3) | 0 | 2 (0.2, 0.4 : essais manuels) |
| 1 — Fiabilité | 0 | 2 (1.1, 1.2) | 5 |
| 2 — Sources | 0 | 4 | 3 |
| 3 — Production | 0 | 3 | 4 |
| 4 — Confidentialité | 0 | 1 | 4 |
| 5 — Distribution | 0 | 1 | 7 |

Seuls 0.1 et 0.3 sont terminés ; la phase 1 est la plus avancée : la vérification des citations (1.1) couvre déjà l'essentiel. Le prochain gain vient surtout de ce qui sert de socle aux autres phases (mesure, tests, installation).

### Ordre recommandé

**A. Fermer la release (≈ 3 jours) — en grande partie fait**
1. ✅ **0.1** : `docs/CHECKLIST_RELEASE.md`, `docs/MCP_PACK_SIGNATURE.md` et `docs/MCP_AUTORISATION.md` écrits d'après le code.
2. ✅ **0.3** : Dependabot ajouté ; l'audit ne bloque plus les PR et ouvre une issue lors de l'exécution hebdomadaire.
3. ⬜ **Valider sur PISTE réel** la recherche dans les codes et le droit à date (`cargo test --lib live_piste -- --ignored`), puis publier. Voir B.1.
4. ⬜ **0.2 et 0.4** : essais manuels sur installeurs Linux et Windows (suivre la checklist), contrôle de `checksums.txt.minisig` sur la dernière release du pack.

**B. Phase 1 — poser le socle de mesure (v1.2)**
1. **1.7 Tests de contrat** — à faire en premier, c'est le plus rapide et il sert tout de suite : enregistrer de vraies réponses PISTE (recherche d'article, recherche plein texte, décision Judilibre, facette Conseil d'État) et les rejouer dans `cargo test`. Les tests actuels utilisent des réponses écrites à la main, qui valident la logique mais pas la forme réelle de l'API.
2. **1.6 Jeu d'évaluation** — commencer *maintenant* la constitution des 50 premières questions avec deux ou trois juristes ; c'est un travail humain qui ne demande pas de code, et qui conditionne les phases 3 et 4.
3. **1.4 Longueur** — une amorce existe : la fenêtre de contexte des modèles est déjà lue dans `App.tsx` mais n'est pas exploitée. Première étape peu coûteuse : estimation du nombre de jetons avant envoi, avec avertissement. Le traitement par morceaux viendra ensuite.
4. ✅ **1.1 (suite)** — ECLI, Conseil d'État, juridictions administratives et cours d'appel ajoutés aux citations vérifiées (code et tests faits ; reste l'essai sur PISTE réel).
5. **1.2 Droit à date donnée (suite)** — le socle est posé pour les codes ; étendre aux lois et décrets (versions LODA) et à la visionneuse.
6. **1.5 Réponses sourcées** — après 1.6, pour pouvoir mesurer l'effet.

**C. Phase 2 — élargir les sources (v1.3), dans cet ordre**
1. **2.2** : lire aussi côté Légifrance les facettes depuis l'API (la taxonomie Judilibre est déjà lue).
2. **2.1** : ajouter `JADE` (cours administratives d'appel, tribunaux administratifs) à côté de `CETAT`, avec une entrée claire « Juridiction administrative ».
3. **2.6** : citation collée, qui s'appuie sur les extracteurs de références de `verify_ext.rs`.
4. **2.3 à 2.5, 2.7** ensuite, selon les retours des juristes testeurs.

**D. En parallèle, dès la v1.2 — Phase 5**
1. **5.3 Vitest** : un seul lanceur avec quelques tests sur le parsing des références et les helpers de recherche suffit pour démarrer ; `tsc --noEmit` ne détecte pas les erreurs de comportement.
2. **5.1 macOS** : à lancer tôt, la signature et la notarisation prennent du temps de calendrier.
3. **5.4 Première configuration guidée** : principal obstacle à l'adoption (identifiants PISTE).

**E. À ne pas engager avant la validation auprès des utilisateurs**
Phases 3 (hors 3.6) et 4. Deux exceptions à avancer plus tôt : **3.6** (OCR et `.eml`/`.msg`) car aujourd'hui un PDF scanné est simplement refusé, et **4.1** (pseudonymisation) si les premiers juristes testeurs le placent en tête.

### Points d'attention constatés pendant le relevé

- **Corrigé le 27 octobre** : `scripts/sign-mcp-pack-release.sh` signait avec le commentaire « juridix-mcp-pack v0.3.4 », alors que l'application exige le mot `version:0.3.4` : une release signée par ce script aurait été **refusée à l'installation**. Le script produit désormais le bon format et s'auto-contrôle ; vérifié avec la fonction de vérification de l'application.
- `src/secureStorage.ts` renvoie vers `docs/ROADMAP.md, chantier 1` (ancien découpage de la roadmap, aujourd'hui sans « chantier 1 ») : le message est à reformuler.
- Les deux liens morts de la phase 0 sont dans `release.yml` (texte du brouillon de release) et dans `CHANGELOG.md` ; ils seront visibles par les utilisateurs dès la publication.
- La CI vérifie le formatage et `clippy -D warnings` : tout nouveau fichier Rust doit passer par `cargo fmt` avant fusion.
- Le facteur de risque le plus élevé reste 1.7 : une évolution de l'API PISTE peut casser une recherche sans qu'aucun test ne le signale.
