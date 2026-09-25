# d2bd-runtime - unit-test audit
tests: 458 (census; 454 real fns - 4 are `#[test]` occurrences inside comments) · src files: 54
net: -6 tests, -60 lines

## Findings (biggest net first)
- duplicate: `resize_retry_with_same_op_id_replays_cached_ack` (src/exec_session.rs:3150) - covered by `control_op_retry_with_same_op_id_replays_cached_ack`（src/exec_session.rs:3095). Same control-op opId dedup ring (`cached_control`/`remember_control`) shared by both the Signal And Resize inline branches; retry at same opId must replay the cached ack without re-issuing to guest.



- duplicate: `validate_lock_parent_accepts_production_tmpfile_shape`（src/runtime_process.rs:774) - covered by `validate_lock_parent_test_mode_accepts_either_0755_or_0750_or_0770`（src/runtime_process.rs:816). Same accept path (`expect_root_owned_parent=false`;0o770 = the tested production-shape mode) fully subsumed by the 3-mode accept loop.





- duplicate: `comm_with_paren`（src/readiness.rs:415) - covered by `comm_with_spaces_and_paren`（src/readiness.rs:423). Both prove `rfind(')'))` handles ')' in comm;the spaces variant covers strictly more (parens + spaces in comm).

- duplicate: `no_paren_at_all`（src/readiness.rs:436) - covered by `empty_input`（src/readiness.rs:443). Missing comm close paren → ParseFailed along the identical rfind-None branch; empty input is the cleaner degenerate case.




- duplicate: `simple_running`（src/readiness.rs:409) - covered by `simple_zombie`（src/readiness.rs:403). Well-formed `/proc/<pid>/stat` line → Alive(state char)（identical pass-through branch; zombie case is the product-relevant one）. 

- duplicate: `dead_process`（src/readiness.rs:449) - covered by `simple_zombie`（src/readiness.rs:403). Same well-formed→Alive(state char) behavior。





- gap: `resource_api` non-list request parsers fail-closed error paths（`parse_resource_names`, `parse_typed_filters`, `typed_filter`, `aliased_cursor`, `aliased_page_size`, `parse_projection` - src/resource_api.rs:415-620) - no unit test anywhere in the crate (only `parse_list_request` is tested, via resource_runtime_support.rs's tests); malformed payloads must fail RequestInvalid/CapabilityUnavailable. cross-check: d2bd/tests may exercise list/get/watch via CLI.



- gap: `broker_transport` mode-bound broker dispatch fail-closed（`validate_instance` → SocketPath/InstanceMismatch;`dispatch` → RequestDenied - src/broker_transport.rs:243-270) and broker ask-deadline timeout（`broker_remaining_before_op` → InternalBrokerTimeout - :69) - zero-test file; these pinchthe daemon's broker request path羽 cross-check: d2bd/tests may cover via integration.

 

- gap: `resource_operator_activation` `select_wave6_resources`/`select_authenticated_resource` error paths（ResourceSelection/ProviderRoute - src/resource_operator_activation.rs:296-380) - zero-test 382-line module; wave-6 activation fail-closed is a security-relevant resource boundary. cross-check: d2bd/tests may cover via integration.

 

## Keep
448 keep fns（families per file）:
- admission.rs 3 - lifecycle-group grant, host-shutdown→HostShutdownUid role mapping, host-shutdown verb allowlist is stop-only.

 
- authority_persistence.rs 9 - ledger prepare admission/refusals、 nonce freshness per prepare, concurrent-prepare single-claim, external-NIC inventory identity, capability/row state machine, durable reservation lifecycle with effect/close/release discipline.+


- autostart.rs 9 - autostart eligibility by runtime/manifest flag, net-before-workload plan ordering, parallelism cap+zero-clamp, failure→Degraded isolation, rerun idempotency, NotAutostart non-gating, outcome predicatesd+


- ch_api.rs 2 - capped blocking read rejects oversized response;, lifecycle states parse cleanlyd+


- ch_stats.rs 11 - # HELP/# TYPE headers, vm/env/role-only labels + no raw names, api_up zero on missing socket, one-hot known-state emission + running/vcpu/memory values,, unknown-state fallback,, absent vcpu/memory lines omitted,, vm.info parse extraction + missing-field tolerance,, 5xx rejection + body-after-CRLF split,, input-order renderingd+


- component_session_vsock.rs 20 - handshake/connect-id line, non-absolute socket/root refusals, directory-owner/world-write + tmp ancestor policy, traversal/symlink/hardlink/missing/not-socket/canonical-parent escapes, peer-credential mismatch,, a k malformed/non-numeric/EOF/too-long/timeout + single-deadline drip,, base-socket-not-suffixed selectiond+


- concurrency.rs 8 - semaphore cap/clamp/thread-exit release,, op-lock read-only no-lock, per-VM serialize vs cross-VM parallel, global-excludes-per-VM,, no-deadlock restart-under-held-guardd+


- console_session.rs 14 - attach handle/kind/offset+owner-uid, unknown-vm none, close/remove client+uid cleanup, read-output after push + stale handle, register-replaces-session, dropped-bytes detection,, ownership matrix (own/deny/admin/stale/admin-bypass)h+


- daemon_audit.rs 20 - api-ready-timeout record shape + disk JSONL match, exec/shell/detached/workload-launch event leak-safety + closed key sets,, health ok/degraded/unavailable-without-path-leak + unique scratch cleanup,, unwritable-destination error + captured-empty,, tail hash read,, no-op writes nothing + authoritative events captured,, api-ready-state roundtrip,, concurrent-writes valid single-chain JSONL,, async-seat appends-before-return,, retention pruning, parse_ymd malformed dates,, launch-event boundary-only serializationd+


- daemon_config.rs 5 - bridge-failure→pre-degraded set semantics, missing config default paths, strict parsing rejects unknown/legacy fields,, realm controllers loader missing+metadata validation+materialized shape,, realm identity loader missing+secret-material/ref rejection+path-leak-free errorsh+


- daemon_version.rs 12 - restart status paths-match/differ/missing/unreadable/missing-install, banner variants, version-file + status serde roundtrip + unknown-field rejectiond+


- exec_detached.rs 1 - resource-backed detached client routes create/status/execution-ref by resource refh+


- exec_session.rs 38 remain - named-stream client framing/credit/cancel/correlation/real-handshake demux/unallowlisted-control reject, Debug redaction (spec, handle, output), owner-disconnect teardown+cancel, establish-failure clean join,, long-poll head-of-line freedom,, per-op capability/tty gates,, write offset/backpressure/idempotency/fresh-per-op-deadline/oversized-chunk/close idempotency/stdout-stderr separation/resize inline/wait-poll,, session-table caps+ownership+handle-redaction, start rate-limit window,, terminal-reaper TTL + stalled-owner reaph+


- exec_session_real.rs 3 - resource exec connector creates/attaches ephemeral process, rejects non-execution targets,, ephemeral-handle Debug redactionh+


- guest_component_session.rs 2 - descriptor hardlink rejection,, config state-root vs endpoint-root mismatch refusalh+


- guest_mode.rs 3 - boot identity derived+redacted,, guest endpoint policy binds generation/purpose/service/vsock binding,, guest runtime exposes no host-authority surfacesh+


- guest_resource_runtime.rs 7 - session-bound store fences old generation,, target-local type classification,, schema-read/watch rejection for Zone types,, watch-not-wired refusal with stable reason_code/retry-class,, seed admission commit-batch-only + descriptor-scoped + type-scoped,, uid-free envelope normalizationh+


- kernel_module_check.rs 13 - minimal bundle pass, kvm alternative fatal+union summary, kvm_amd alone ok,, graphics→udmabuf required, builtin=y detection, virtiofs conditional on declared node,, usbip/tpm optional-degraded with affected_vms, nvidia warn-only-not-degraded,, builtin counts present,, fatal typed error summary,, /sys/module builtin detection via read_loaded_modules_at,, production /proc/modules path pinsh+


- metrics.rs 17 - inventory names, vm_state kind/labels, start buckets canonical,, render help/type/labels, broker fallback recording,, histogram buckets+sum+count,, gauge override,, label-key allowlist + forbidden high-cardinality keys,, total-series cap,, workload metrics leak-free,, handler GET text / 404 / 405,, wrong-kind panic,, identity-label drop + canonical ordering,, ch_stats append + its 404h+


- otel_host_bridge_readiness.rs 14 - readiness matrix (ready/pending×2/failed×2/precedence×2), wrapper transitions (ready/timeout/runner-exit degraded), config from_values fallback+strict,, degraded envelope shape + ready-no-typed-errorh+


- ownership_preflight.rs 5 - missing dir clean, drift message axes/kind/stat-failure rendering,, provisioned dir with unresolvable principals cleanh+


- pidfs_probe.rs 5 - available ok,, unsupported hard-typed-error / soft-ok,, unexpected always soft-defers,, live probe no-panic smokeh+


- public_projection.rs 1 - public service states follow pidfd rolesd+


- readiness.rs 9 remain - proc-state parser comma boundaried (zombie, spaces+parens, truncated, empty), unix-socket listen/exists probe,, api-socket live+dead 404 paths,, oneshot zombie shortcut + live timeout, async wait awaits async probe seat + named timeout.

 (4 of the proc-state family deleted as duplicates above; the 8 parse tests exercise a byte-identical local copy of `read_proc_state`'s parse body - a test-quality smell, not a product bug.）


- resource_runtime_support.rs 30 - closed subject/resource type sets,, system-core credential-commit authorization (+wrong subresource/verb denials), zone-local user resolution (+duplicate/stale/uid-mismatch/observed-generation denials), committed Roles/RoleBindings→distinct user grants,, all closed subject types compile,, missing subject invalidates nothing but grants nothing,, tombstone-by-phase matrix,, fingerprint vs unready/missing subject rows,, recreated-uid fencing + same-name user recreation + multi-revision retain-fence/require-rebind matrix,, unknown/cross-zone subject refusal, evidence uid/generation/revision match,, stable uid repeatability + v4 shape,, bundle mutation op id requires zone uid,, core startup handler-gate-before-readiness,, host-resource-count phase matrix,, config cleanup pending counts deleted prior generations only,, completed-watch restart clearance,, system-core subject preserves component registration,, system-core policy evidence admits own lane + refuses foreign class,, List parser (typed pagination/filters, unsupported fields, conflicting aliases,, malformed decode fails closed,, cursor retention,, error envelope kind/retry metadata,, public not-found/plane-unavailable kinds,, public operation id exact-targeth+


- runtime_process.rs 7 remain - sd_notify ready/status pathname+abstract datagrams, unreachable error no-panic,, public socket chgrp to socket-gid even non-root + mode 0660,, test-mode skips chown,, lock-parent mode rejection, 3-mode test accept looph+


- shell_backend.rs 1 - component-session shell error mapping stays closed vocabularyh+


- ssh_host_key_preflight.rs 10 - missing/symlink/non-dir dir drift,, empty dir ok,, .pub+unrelated ignored,, wrong-owner drift, wrong-mode drift with expected 0400,, symlink-key drift,, drift path/reason accessors,, root happy pathh+


- supervisor/dag.rs 15 - topo_sort linear/diamond/cycle/self-loop/unknown-edge/duplicate-node,, executor topo success/fail-fast/propagates-topo-error/budget threading,, split-readiness pass/no-wait-api-stop/strict-fail,, ApiReadyState + report serde contractsd+


- supervisor/pidfd_table.rs 16 - spawn reservation exclusion+release,, live start-time zombie/dead→gone,, register/deregister, conditional deregister newer-preserving,, observe-only dup/peek non-consuming, list-for-vm, duplicate-registration refusal,, subreaper self-test,, register+signal+snapshot+restore roundtrip, stale-ESRCH prune,, wait_terminated timeout+running-kill, broker-reap-log ECHILD path,, reap buffer survives disconnect,, concurrent snapshot race-free + unique tmp paths + union-under-guardh+


- supervisor/readiness_liveness.rs 9 - classify matrix (no-entry, drift-reused, gone+reap, pollin+match, alive, reap-alone, unreadable±pollin), stale-same-role reap pid-match regressionh+


- target_runtime.rs 10 - mode closed + surface matrix,, admission budget cap+release,, session-generation revoke, assignment drop releases slot,, guest cannot admit host/non-provider,, target-local controller launch requires ready session + assignment lifecycle + reconnect new epoch,, parent-session revoke w/o ready session,, cleanup needs verified children + exact repair owner,, quarantined child + finalizer held ,, reconnect → new controller process identityh+


- terminal_session.rs 2 - output Debug redaction,, TerminalBackend shared ops over fake backend (write/read/wait/close failure)h+


- typed_error.rs 14 - internal-* redaction per variant (5), guest-shell-disabled typed+exit code,, workload-launch kinds typed/actionable/redacted,, wire-ifname redacted,, component read/exec/shell failure kinds distinct+leak-free+non-empty,, envelope kind discriminants table,, sshd/bundle-dnsmasq envelope shapesh+


- typed_shell_targets.rs 3 - reserve one-create-seat per uid/name + drop release,, release inside tokio runtime non-panicking,, remember/cached/forget exact targetsd+


- unix_transport.rs 1 - request-fd frame transfers one cloexec descriptorh+


- unsafe_local_helper.rs 19 - outbound wake,, listener cloexec,, socket buffer minimum two-sided exact,, operation-ledger fingerprint conflict per field (argv/accent/target) + bounded completed history + abort-retry + timeout ambiguous→retry-after-retention + late-response reconcile + reconnect-snapshot reconcile,, registry uid isolation + uids hidden in Debug + registration supersede + out-of-eligibility rejection + correlated dispatch w/o uid-in-frame + queue-saturation + heartbeat staleness + per-target failure scopingh+


- vm_start_support.rs 4 - trusted graphics proxy resource-backed; qemu-media proxy stays legacy DAG,, trusted guest frontend resource-backed+guest-owned,, non-graphics role not resource-backedh+


- wire.rs 8 - retired shell/compo-session request rejection,, launcherExec rejects argv + accepts reference-only fields,, resourceRequest payload preserved + Reconcile global-mutating lock class,, typed Guest/Process lifecycle lock per-target,, real audit response roundtrips public contracth+


- workload_dispatch.rs 10 - launch ledger idempotent+changed-fingerprint conflict, per-uid capacity isolation,, route_for_provider unsafe-local never coerced,, host-local realm direct-match,, resolve_exec trusted descriptors only + missing/mismatch/tamper/graphical drift refusal,, resolve_shell bare+canonical compat + unsafe-local fail-closed + known-vm-precedes-alias + unsupported/ambiguous refusalh+


- workload_target_index.rs 16 - build skips/indexes identity, identity_for_vm unknown/present,, canonical resolve ok/not-found-with-message,, legacy fast-path pass-through,, workload-id alias unambiguous/ambiguous-fail-closed, legacy-priority, unknown-fall-through,, alias message names candidates,, restart-simulation identity stability (rebuilt identical, JSON roundtrip preserves, transitional stays absent, mixed preserves explicit only)h+


- zone_authority.rs 7 - two-level zone identity distinct,, tuple mismatch refused before store,, old bundle contract rejected,, incomplete generation set refused,, generation digest order-independent+changes-on-any-zone,, coordinator registers authoritative zones + resolves bindings,, unbound VM auto-registers as own zoneh+


Overlap reference - tests/runtime_boundary.rs (2 compile-boundary tests: no runtime unit-test overlap identified).

## route-out
- `metrics_handler_with_ch_stats` re-implements the GET/method/path validation block of `metrics_handler` byte-for-byte instead of delegating (src/metrics.rs:784-806) - two copies can drift;the ch_stats 404/405 tests only pin their own copy. Not investigated further.
