#!/usr/bin/env bash
# Signe la liste d'empreintes (`checksums.txt`) d'une release du pack MCP avec minisign.
#
# Usage :
#   scripts/sign-mcp-pack-release.sh <tag> <dossier-des-assets> [clé-secrète] [clé-publique.pub]
#
# Exemple :
#   scripts/sign-mcp-pack-release.sh v0.3.1 ./dist ~/.minisign/juridix-mcp-pack.key juridix-mcp-pack.pub
#
# Produit `<dossier>/checksums.txt.minisig`, à joindre à la release GitHub (même nom d'asset).
# Le commentaire signé « juridix-mcp-pack <tag> » lie la signature à CETTE release : Juridix
# refuse la signature d'une autre release (rejeu). Voir docs/MCP_PACK_SIGNATURE.md.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi

TAG="$1"
DIR="$2"
KEY="${3:-$HOME/.minisign/juridix-mcp-pack.key}"
PUB="${4:-}"

command -v minisign >/dev/null 2>&1 || { echo "minisign introuvable (https://jedisct1.github.io/minisign/)" >&2; exit 1; }
[[ "$TAG" =~ ^v[0-9]+(\.[0-9]+)*([-+][0-9A-Za-z.-]+)?$ ]] || { echo "Tag invalide : « $TAG » (attendu : v1.2.3)" >&2; exit 1; }
[ -f "$DIR/checksums.txt" ] || { echo "Fichier introuvable : $DIR/checksums.txt" >&2; exit 1; }
[ -f "$KEY" ] || { echo "Clé secrète introuvable : $KEY" >&2; exit 1; }

# Garde-fou : la liste d'empreintes doit nommer les archives de CETTE version.
VERSION="${TAG#v}"
if ! grep -q -- "-${VERSION}\.zip" "$DIR/checksums.txt"; then
  echo "checksums.txt ne mentionne aucune archive en version ${VERSION} : mauvais tag ou mauvais dossier ?" >&2
  exit 1
fi

rm -f "$DIR/checksums.txt.minisig"
minisign -S -s "$KEY" -m "$DIR/checksums.txt" -x "$DIR/checksums.txt.minisig" -t "juridix-mcp-pack ${TAG}"

if [ -n "$PUB" ]; then
  minisign -V -p "$PUB" -m "$DIR/checksums.txt" -x "$DIR/checksums.txt.minisig"
else
  echo "(vérification ignorée : passez la clé publique en 4e argument pour contrôler la signature produite)"
fi

echo
echo "Signature écrite : $DIR/checksums.txt.minisig"
echo "À joindre à la release : gh release upload ${TAG} \"$DIR/checksums.txt.minisig\" --clobber"
