# Manage stacked wave pull requests with Git Town

Use this procedure to create or update an ordinary GitHub pull-request stack.
Run every command from a dedicated clean worktree after entering `nix develop`.
The examples use `delivery-contracts` as the root feature branch and
`delivery-runtime` as its child.

## Verify the delivery environment

Select the repository and run the fail-closed capability probe:

```console
repository=github.com/example/d2b
nix run .#delivery -- stack capability --repository "$repository"
```

Stop if the probe reports an unsupported Git Town version, missing
noninteractive flags, failed GitHub authentication, missing repository access,
or an unavailable ordinary pull-request API.

## Configure the topology

Define guards that reject dirty worktrees and missing local branches. Do not
replace these checks with a fetch or an inferred remote parent: the exact local
refs are the topology authority used by delivery tooling.

```console
require_clean() {
  test -z "$(git status --porcelain=v1 --untracked-files=all)" || {
    echo "refusing delivery from a dirty worktree" >&2
    return 1
  }
}
require_branch() {
  git show-ref --verify --quiet "refs/heads/$1" || {
    echo "missing local branch: $1" >&2
    return 1
  }
}
set_parent() {
  branch=$1
  parent=$2
  require_clean
  require_branch "$branch"
  require_branch "$parent"
  git switch "$branch"
  git town set-parent "$parent" --non-interactive
  test "$(git town config get-parent "$branch")" = "$parent" || {
    echo "Git Town parent verification failed: $branch -> $parent" >&2
    return 1
  }
}

require_clean
require_branch main
git config --local git-town.main-branch main
set_parent delivery-contracts main
set_parent delivery-runtime delivery-contracts
```

Each feature branch has exactly one immediate parent. Repeat `set_parent` from
root to leaf for additional dependent branches. Independent branches use
`main`, not another feature branch, as their parent.

## Propose the ordinary pull requests

From the leaf branch, synchronize the complete stack without interactive
prompts or automatic conflict resolution, then create or update its ordinary
GitHub pull requests without opening a browser:

```console
require_clean
git switch delivery-runtime
git town sync --stack --non-interactive --no-auto-resolve
require_clean
git town propose --stack --non-interactive --no-browser --no-auto-resolve
```

If synchronization encounters a conflict, stop and resolve it explicitly.
Commit the resolution, rerun the clean-worktree guard, and repeat the commands.
Do not submit a partially synchronized stack.

## Update an existing stack

After committing changes to any branch, switch to the leaf and repeat the same
root-to-leaf synchronization and proposal update:

```console
require_clean
git switch delivery-runtime
git town sync --stack --non-interactive --no-auto-resolve
require_clean
git town propose --stack --non-interactive --no-browser --no-auto-resolve
```

Create the immutable delivery snapshot only after Git Town has updated every
ordinary pull request and its immediate base.

## Retarget a dependent pull request

To move `delivery-runtime` directly onto `main`, update Git Town's parent,
verify it, synchronize, and let Git Town update the ordinary pull-request base:

```console
set_parent delivery-runtime main
require_clean
git town sync --stack --non-interactive --no-auto-resolve
require_clean
git town propose --stack --non-interactive --no-browser --no-auto-resolve
test "$(git town config get-parent delivery-runtime)" = main
```

Use the same sequence with another existing local branch name to retarget onto
a different parent. A missing parent, dirty worktree, synchronization conflict,
or parent-verification mismatch is a hard stop. Never repair topology with an
ad-hoc `gh` API mutation or direct base edit.

After any retarget, run the delivery history proof and required CI on the new
history before reusing eligible external panel records. Follow the snapshot,
validation, seal, and merge contracts in
[Delivery tooling](../reference/delivery-tooling.md).
