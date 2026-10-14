#!/bin/sh
# Record resolved build/runtime packages without adding runtime dependencies.
set -eu
if [ "$#" -lt 3 ]; then
    echo 'Usage: inventory.sh OUTPUT_DIRECTORY STAGE BASE_IMAGE [BASE_IMAGE ...]' >&2
    exit 2
fi
output=$1
stage=$2
shift 2
for image in "$@"; do
    case "$image" in
        *@sha256:*) ;;
        *) echo 'Inventory base images must include a SHA-256 digest.' >&2; exit 2 ;;
    esac
    printf '%s' "${image##*@sha256:}" | grep -Eq '^[0-9a-f]{64}$' || exit 2
done
mkdir -p "$output"
{
    printf 'schema=qspquest.container-inventory/v1\n'
    printf 'stage=%s\n' "$stage"
    printf 'recorded_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'architecture=%s\n' "$(dpkg --print-architecture)"
} > "$output/metadata.txt"
printf '%s\n' "$@" > "$output/base-images.txt"
cp /etc/os-release "$output/os-release"
LC_ALL=C dpkg-query -W -f='${binary:Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' \
    > "$output/os-packages.unsorted"
LC_ALL=C sort "$output/os-packages.unsorted" > "$output/os-packages.tsv"
rm "$output/os-packages.unsorted"
# Keep the complete package denominator, including removed/configuration-only rows.
# Copyright documents are needed only for currently installed package records.
mkdir -p "$output/os-copyright"
: > "$output/os-copyright.tsv"
missing=0
tab=$(printf '\t')
while IFS="$tab" read -r package version architecture status; do
    printf '%s' "$package" | LC_ALL=C grep -Eq '^[a-z0-9][a-z0-9.+-]*(:[a-z0-9][a-z0-9-]*)?$' || {
        echo 'Invalid Debian package coordinate.' >&2; exit 2;
    }
    [ "$status" = installed ] || continue
    document="/usr/share/doc/${package%%:*}/copyright"
    suffix=copyright
    if [ ! -s "$document" ] && [ -s "$document.gz" ]; then
        document="$document.gz"
        suffix=copyright.gz
    fi
    if [ ! -s "$document" ]; then
        printf '%s\t%s\t%s\tmissing\t\t\t\n' "$package" "$version" "$architecture" >> "$output/os-copyright.tsv"
        missing=$((missing + 1))
        continue
    fi
    before=$(sha256sum "$document")
    before=${before%% *}
    # Debian names cannot contain underscores, so this maps multiarch ':' names
    # without collisions or platform-specific path characters.
    path_package=$(printf '%s' "$package" | tr ':' '_')
    relative="os-copyright/$path_package/$suffix"
    mkdir -p "$output/os-copyright/$path_package"
    cp "$document" "$output/$relative"
    after=$(sha256sum "$document")
    after=${after%% *}
    copied=$(sha256sum "$output/$relative")
    copied=${copied%% *}
    if [ "$before" != "$after" ] || [ "$before" != "$copied" ]; then
        echo 'Package copyright document changed during collection.' >&2
        exit 2
    fi
    printf '%s\t%s\t%s\tretained\t%s\t%s\t%s\n' "$package" "$version" "$architecture" "$document" "$relative" "$copied" >> "$output/os-copyright.tsv"
done < "$output/os-packages.tsv"
printf 'installed_copyright_documents_missing=%s\n' "$missing" >> "$output/metadata.txt"
if [ -n "${INVENTORY_PYTHON:-}" ]; then
    "$INVENTORY_PYTHON" --version > "$output/python-version.txt"
    "$INVENTORY_PYTHON" -m pip freeze --all > "$output/python-packages.txt"
fi
if command -v rustc >/dev/null 2>&1; then
    rustc --version --verbose > "$output/rustc.txt"
fi
LC_ALL=C dpkg-query -W -f='${binary:Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n' \
    > "$output/os-packages.final.unsorted"
LC_ALL=C sort "$output/os-packages.final.unsorted" > "$output/os-packages.final.tsv"
if ! cmp -s "$output/os-packages.tsv" "$output/os-packages.final.tsv"; then
    echo 'Package database changed during inventory collection; both tables were retained.' >&2
    exit 2
fi
rm "$output/os-packages.final.unsorted" "$output/os-packages.final.tsv"
if [ "$missing" -ne 0 ]; then
    echo 'Installed package copyright inventory is incomplete; missing records were retained.' >&2
    exit 2
fi
