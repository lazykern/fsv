#!/bin/sh
# Regenerate crates/fsv-api/src/testdata/*_oracle.json from a pinned Python export.
#   git -C <fsv> archive <sha> | tar -x -C /tmp/fsv-py-<sha>
#   FSV_PY_SRC=/tmp/fsv-py-<sha> FSV_PY_COMMIT=<full sha> FSV_PY_BIN=<python with fsv deps> port/golden/regen_api.sh
# Journal timestamps ("sent at <ts>") are normalised to "an earlier run": the Rust
# in-memory journal rows carry no ts.
set -e
here=$(cd "$(dirname "$0")" && pwd)
out=$here/../../crates/fsv-api/src/testdata
export PYTHONPATH=$FSV_PY_SRC/src:$here
cd "$FSV_PY_SRC"
for g in "$here"/gen_fsv_api_*.py; do
  n=$(basename "$g" .py); n=${n#gen_fsv_api_}
  [ "$n" = query_flow_matrix ] && n=query_flow
  [ -f "$out/${n}_oracle.json" ] || continue
  "${FSV_PY_BIN:-python}" "$g" 2>/dev/null |
    perl -pe 's/(?<= at )\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d{4}/an earlier run/g' > "$out/${n}_oracle.json.new" &&
    [ -s "$out/${n}_oracle.json.new" ] && mv "$out/${n}_oracle.json.new" "$out/${n}_oracle.json" ||
    { echo "skipped $n" >&2; rm -f "$out/${n}_oracle.json.new"; }
done
