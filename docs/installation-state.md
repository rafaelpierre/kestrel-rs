# Installation state

`kestrel skill install` records paths in `~/.kestrelsearch/config.toml`;
`kestrel skill uninstall` reads those paths and removes selected or stale records.
The schema, command arguments, and installation locations are unchanged.

Each record update opens a persistent `config.toml.lock` sidecar, acquires an
exclusive OS file lock, and only then loads, edits, and saves the TOML. Contention
is retried for up to ten seconds before returning an I/O timeout on stderr with a
nonzero command exit status. Retry after the other installation finishes. The OS
releases the lock when its handle closes or the process exits. Do not delete the
sidecar: another process could otherwise lock a different file at the same path.

Updates stage the serialized document in a temporary file in the destination's
own directory, preserve existing permissions, sync the staged file, and atomically
replace the destination. New configs use tempfile's private permissions (0600 on
Unix). On Unix, the containing directory is then synced. Failures before replacement
leave the old config byte-for-byte intact; invalid TOML is reported and never
overwritten. A failure syncing the directory is reported **after** replacement,
so the new contents may already be visible despite the error. Read-only listing
does not take a lock: it sees either complete version when cooperating writers
replace the file.

Existing config symlinks are resolved and retained; the destination and its
adjacent lock are used. Dangling config symlinks are rejected without replacing
them. Parent directory aliases resolve to the same lock. External concurrent
symlink retargeting is unsupported. Atomic replacement changes the file identity;
hard-link aliases are not updated together, and ownership, ACLs and extended
attributes are not promised to survive replacement (permission bits are preserved).

These guarantees require cooperating versions of Kestrel and filesystem support
for OS locking and atomic replacement. Older binaries and editors that ignore
the lock can still race. Network filesystems, hardware failure and universal
power-loss durability are outside the guarantee; non-Unix builds do not perform
the final directory sync. Abrupt termination can leave an unused temporary file,
but it does not truncate the old config.

The transaction covers config bookkeeping, not the skill files themselves.
Installation writes `SKILL.md` before recording it; uninstallation removes the
file before removing its record. Failure can therefore leave an unrecorded file
or stale record. Inspect the intended paths before retrying. Restore or repair
already malformed TOML explicitly; Kestrel does not discard it automatically.

## Validation

`src/config/tests.rs` runs the real config store in independent child processes
with isolated paths. It covers first-time creation, concurrent addition/removal,
unrelated keys/comments, contention, lock release after process termination,
faults before replacement, malformed input, permissions, and symlinks. CLI tests
install the generated skill into a temporary project with a temporary home and
verify its installation-state guidance. No live providers or user config are
needed. Locking uses the standard library API available since Rust 1.89 and
staging reuses the existing `tempfile` dependency.
