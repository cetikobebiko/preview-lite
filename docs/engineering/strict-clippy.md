# Strict Clippy Gate

`cargo clippy --locked --all-targets --all-features -- -D warnings` is a required CI gate.
It runs in the pull-request/push workflow and in the release workflow from `src-tauri`.

## 2026-07-29 cleanup report

The initial strict library pass reported 66 diagnostics. The separate
`document_io_fs` integration target also reported five dead-code diagnostics because it compiles
the production atomic-I/O module without executing its PDFium entry points.

| Category | Initial count | Resolution |
|---|---:|---|
| `dead_code` | 3 library + 5 filesystem-target | Removed unreachable raw PDF commands; filesystem target now type-checks its intentionally unexecuted PDFium entry points. |
| `result_large_err` | 49 | Boxed the optional session snapshot and atomic-recovery path, preserving the serialized IPC shape while making error results small. |
| `too_many_arguments` | 4 | Retained only Tauri command signatures, each with a nearby explanation and narrow lint allowance. |
| `for_kv_map` | 1 | Iterates map values directly. |
| `unnecessary_get_then_check` | 1 | Uses `contains_key`. |
| `needless_borrow` | 3 | Removed redundant references. |
| `explicit_counter_loop` | 1 | Uses `enumerate`. |
| `unnecessary_cast` | 1 | Removed the redundant `usize` cast. |
| `manual_flatten` | 1 | Uses iterator flattening. |
| `new_without_default` | 1 | Added `Default` for `PluginState`. |
| `manual_rem_euclid` | 1 | Uses `rem_euclid`. |

No file- or module-wide Clippy allowances were added. The only `too_many_arguments` allowances
are attached to Tauri command functions whose top-level argument layout is the stable IPC contract.
