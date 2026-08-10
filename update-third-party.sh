#!/usr/bin/env bash
set -euo pipefail

# Dependency versions — pin to tagged nRF Connect SDK releases, not moving branches.
NRFXLIB_REPO="https://github.com/nrfconnect/sdk-nrfxlib.git"
NRFXLIB_REF="v3.4.0"

# sdk-nrf ships the sample application this is built for, including a generic secondary-cell
# battery model ("Example") meant for bring-up testing before you have a real, profiled model.
NRF_REPO="https://github.com/nrfconnect/sdk-nrf.git"
NRF_REF="v3.4.0"

THIRD_PARTY="nrf-fuel-gauge-sys/third_party"

sparse_clone() {
    local repo="$1"
    local ref="$2"
    local dest="$3"
    shift 3
    local dirs=("$@")

    echo "Fetching $repo @ $ref -> $dest"

    rm -rf "$dest"
    mkdir -p "$dest"

    git -C "$dest" init -q
    git -C "$dest" remote add origin "$repo"
    git -C "$dest" sparse-checkout init --cone
    git -C "$dest" sparse-checkout set "${dirs[@]}"
    git -C "$dest" fetch --filter=blob:none --depth 1 origin "$ref"
    git -C "$dest" checkout FETCH_HEAD
    rm -rf "$dest/.git"

    echo "  Done."
}

# Delete all files in a directory except those named in the remaining arguments
keep_only() {
    local dir="$1"
    shift
    local keep=("$@")

    local find_args=()
    for f in "${keep[@]}"; do
        find_args+=( ! -name "$f" )
    done
    find "$dir" -maxdepth 1 -type f "${find_args[@]}" -delete
}

# ── nrfxlib: nrf_fuel_gauge header, license, and the one lib variant this crate supports
#    (Cortex-M4, hard-float, secondary-cell) ──
sparse_clone "$NRFXLIB_REPO" "$NRFXLIB_REF" "$THIRD_PARTY/nordic/nrfxlib" \
    nrf_fuel_gauge/include \
    nrf_fuel_gauge/lib/cortex-m4/hard-float \
    nrf_fuel_gauge/license.txt

# Remove top-level repo files we don't need (Jenkinsfile, CODEOWNERS, lint configs, etc.);
# cone-mode sparse-checkout always includes these regardless of the requested dirs.
keep_only "$THIRD_PARTY/nordic/nrfxlib" LICENSE

# Drop the primary-cell (non-rechargeable) variant and every other target/float-ABI directory —
# this crate only ever links libnrf_fuel_gauge.a for cortex-m4/hard-float.
rm -f "$THIRD_PARTY/nordic/nrfxlib/nrf_fuel_gauge/lib/cortex-m4/hard-float/libnrf_fuel_gauge_primary.a"

# Remove the primary-cell battery model .inc files bundled by Nordic — irrelevant to the
# secondary-cell-only variant this crate links, and this crate uses your own model instead.
rm -rf "$THIRD_PARTY/nordic/nrfxlib/nrf_fuel_gauge/include/battery_models"

# ── sdk-nrf: the nPM1300 fuel gauge sample's generic "Example" battery model — used only as
#    this crate's out-of-the-box test model until you supply your own via
#    NRF_FUEL_GAUGE_MODEL_PATH. Sparse-checkout is directory-granular, so grab the sample's
#    whole src/ dir and then keep_only the one file we actually want. ──
sparse_clone "$NRF_REPO" "$NRF_REF" "$THIRD_PARTY/nordic/nrf" \
    samples/pmic/native/npm13xx_fuel_gauge/src

keep_only "$THIRD_PARTY/nordic/nrf" LICENSE
# Cone-mode sparse-checkout includes every ancestor directory's own files along the requested
# path (samples/CMakeLists.txt, .../npm13xx_fuel_gauge/prj.conf, etc.), not just the leaf dir —
# strip all of that recursively, keeping only the one file we actually want.
find "$THIRD_PARTY/nordic/nrf/samples" -type f ! -name battery_model.inc -delete
find "$THIRD_PARTY/nordic/nrf/samples" -type d -empty -delete

# Verify key files exist
echo ""
echo "Verifying fetched files..."
MISSING=0
for f in \
    "$THIRD_PARTY/nordic/nrfxlib/nrf_fuel_gauge/license.txt" \
    "$THIRD_PARTY/nordic/nrfxlib/nrf_fuel_gauge/include/nrf_fuel_gauge.h" \
    "$THIRD_PARTY/nordic/nrfxlib/nrf_fuel_gauge/lib/cortex-m4/hard-float/libnrf_fuel_gauge.a" \
    "$THIRD_PARTY/nordic/nrf/samples/pmic/native/npm13xx_fuel_gauge/src/battery_model.inc" \
; do
    if [ ! -f "$f" ]; then
        echo "  MISSING: $f"
        MISSING=1
    fi
done

if [ "$MISSING" -eq 0 ]; then
    echo "All key files present."
else
    echo "ERROR: Some expected files are missing!"
    exit 1
fi
