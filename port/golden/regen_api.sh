#!/bin/sh
# Regenerate the fsv-api and fsv-flows oracles from a pinned Python export.
#   git -C <fsv> archive <sha> | tar -x -C /tmp/fsv-py-<sha>
#   FSV_PY_SRC=/tmp/fsv-py-<sha> FSV_PY_COMMIT=<full sha> FSV_PY_BIN=<python with fsv deps> \
#       port/golden/regen_api.sh
# FSV_PY_BIN may be any interpreter that has the fsv dependencies; the export's
# own src/ goes first on PYTHONPATH. Journal timestamps ("sent at <ts>") are
# normalised to "an earlier run": the Rust in-memory journal rows carry no ts.
# Generators that fail on the pinned Python are reported and their fixture is
# left alone.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$here/../..
out=$root/crates/fsv-api/src/testdata
py=${FSV_PY_BIN:-python}
export FSV_PY_SRC FSV_PY_COMMIT
export PYTHONPATH=$FSV_PY_SRC/src:$here:$here/oracles/fsv_oracle
norm() { perl -pe 's/(?<= at )\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d{4}/an earlier run/g'; }

run() { # script fixture [args...]
  s=$1; f=$2; shift 2
  (cd "$FSV_PY_SRC" && "$py" "$s" "$@" 2>/dev/null) | norm > "$f.new"
  if [ -s "$f.new" ]; then mv "$f.new" "$f"; else rm -f "$f.new"; echo "skipped $f" >&2; fi
}

for g in "$here"/gen_fsv_api_*.py; do
  n=$(basename "$g" .py); n=${n#gen_fsv_api_}
  [ "$n" = query_flow_matrix ] && n=query_flow
  [ -f "$out/${n}_oracle.json" ] || continue
  run "$g" "$out/${n}_oracle.json"
done
run "$here/gen_fsv_api_ticket_lifecycle.py" "$out/ticket_lifecycle_ack_oracle.json" --ack
run "$here/oracles/fsv_oracle/o_create.py" "$out/create_oracle.json"
run "$here/oracles/fsv_oracle/o_catalog.py" "$out/catalog_oracle.json"
tmp=$(mktemp -d)
(cd "$FSV_PY_SRC" && "$py" "$here/oracles/fsv_api_views/oracle.py" "$out/views_oracle.json" "$tmp/properties_change_501.json" >/dev/null 2>&1) \
  || echo "skipped views_oracle.json" >&2
rm -rf "$tmp"
