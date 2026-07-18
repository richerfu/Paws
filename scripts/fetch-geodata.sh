#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RAW_GEODATA_DIR="${RAW_GEODATA_DIR:-$ROOT_DIR/entry/src/main/resources/rawfile/geodata}"
BASE_URL="${GEODATA_BASE_URL:-https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest}"

COUNTRY_URL="${COUNTRY_URL:-$BASE_URL/country.mmdb}"
ASN_URL="${ASN_URL:-$BASE_URL/GeoLite2-ASN.mmdb}"
GEOSITE_DAT_URL="${GEOSITE_DAT_URL:-$BASE_URL/geosite.dat}"

mkdir -p "$RAW_GEODATA_DIR"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

download() {
  local url="$1"
  local dest="$2"
  local tmp="$tmp_dir/$(basename "$dest").download"
  if ! curl --fail --location --show-error --silent "$url" --output "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  if [[ ! -s "$tmp" ]]; then
    echo "Downloaded empty geodata file from $url" >&2
    return 1
  fi
  mv "$tmp" "$dest"
}

echo "Writing geodata rawfiles to $RAW_GEODATA_DIR"
download "$COUNTRY_URL" "$RAW_GEODATA_DIR/Country.mmdb"
download "$ASN_URL" "$RAW_GEODATA_DIR/GeoLite2-ASN.mmdb"

download "$GEOSITE_DAT_URL" "$RAW_GEODATA_DIR/geosite.dat"

for file in Country.mmdb GeoLite2-ASN.mmdb geosite.dat; do
  if [[ ! -s "$RAW_GEODATA_DIR/$file" ]]; then
    echo "Missing expected geodata rawfile: $RAW_GEODATA_DIR/$file" >&2
    exit 1
  fi
done

du -h "$RAW_GEODATA_DIR"/Country.mmdb "$RAW_GEODATA_DIR"/GeoLite2-ASN.mmdb "$RAW_GEODATA_DIR"/geosite.dat
