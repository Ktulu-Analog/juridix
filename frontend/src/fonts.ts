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

// Polices et feuilles de style EMBARQUÉES dans l'application.
//
// Juridix ne doit contacter aucun service tiers (Google Fonts, jsDelivr…) :
// ces ressources étaient auparavant chargées depuis des CDN, ce qui exposait
// l'adresse IP et l'usage de l'application, et imposait de les autoriser dans
// la CSP. Elles sont désormais empaquetées par Vite (paquets npm @fontsource/*
// et katex) et servies depuis l'origine de l'application (`font-src 'self'`).
//
// Sous-ensembles « latin » et « latin-ext » uniquement (français et langues
// d'Europe occidentale/centrale). Mêmes graisses qu'auparavant.

// Sora — titres (--font-display)
import '@fontsource/sora/latin-400.css';
import '@fontsource/sora/latin-500.css';
import '@fontsource/sora/latin-600.css';
import '@fontsource/sora/latin-ext-400.css';
import '@fontsource/sora/latin-ext-500.css';
import '@fontsource/sora/latin-ext-600.css';

// DM Sans — texte courant (--font-body)
import '@fontsource/dm-sans/latin-400.css';
import '@fontsource/dm-sans/latin-500.css';
import '@fontsource/dm-sans/latin-600.css';
import '@fontsource/dm-sans/latin-400-italic.css';
import '@fontsource/dm-sans/latin-ext-400.css';
import '@fontsource/dm-sans/latin-ext-500.css';
import '@fontsource/dm-sans/latin-ext-600.css';
import '@fontsource/dm-sans/latin-ext-400-italic.css';

// JetBrains Mono — code (--font-mono)
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-500.css';
import '@fontsource/jetbrains-mono/latin-ext-400.css';
import '@fontsource/jetbrains-mono/latin-ext-500.css';

// Inter et Ubuntu — proposées dans Réglages > Apparence
import '@fontsource/inter/latin-400.css';
import '@fontsource/inter/latin-500.css';
import '@fontsource/inter/latin-600.css';
import '@fontsource/inter/latin-ext-400.css';
import '@fontsource/inter/latin-ext-500.css';
import '@fontsource/inter/latin-ext-600.css';
import '@fontsource/ubuntu/latin-400.css';
import '@fontsource/ubuntu/latin-500.css';
import '@fontsource/ubuntu/latin-700.css';
import '@fontsource/ubuntu/latin-ext-400.css';
import '@fontsource/ubuntu/latin-ext-500.css';
import '@fontsource/ubuntu/latin-ext-700.css';

// KaTeX (rendu des formules) — feuille de style + polices, auparavant sur jsDelivr
import 'katex/dist/katex.min.css';
