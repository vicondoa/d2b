### Security

- The heavy-gate semaphore now requires a root-provisioned runtime namespace
  whose directories cannot be renamed by either a peer or the invoking user,
  fails closed with no weaker fallback when that namespace is unavailable, and
  performs every slot operation relative to pinned directory descriptors.
  Self-guard regression tests also start from an empty environment and close
  inherited descriptors, so parent gate state cannot silently authorize a test
  child.
- The heavy-entrypoint census now accepts source relationships only from
  executable regular shell entrypoints. Inert sibling text can no longer claim
  to source a heavy script and hide that script from the self-guard check.
