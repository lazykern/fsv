# Parity ledger

Each section: Python module → Rust location, status, ported tests, golden
fixtures, known gaps. Statuses: `ported`, `partial`, `missing`.

## fsv-py

| Python behavior | Rust | status |
|---|---|---|
| `json.dumps(...)` all option combos used by fsv | `fsv_py::Dumps`, `canonical`, `compact`, `pretty` | ported (unit tests vs CPython output) |
| `float.__repr__` | `fsv_py::float_repr` | ported |
| `str()` / `repr()` of JSON values | `fsv_py::py_str`, `py_repr`, `py_repr_str` | ported |

## Divergences

- Plain (non-table) terminal text is not soft-wrapped at terminal width.
