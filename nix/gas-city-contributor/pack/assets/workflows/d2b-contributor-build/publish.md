Publish only when the run's explicit `push` and `open_pr` inputs are enabled.
The workflow must not infer publication intent from a review, decision, or
branch name.

The main service invokes the immutable helper with the current managed
worktree.  The helper checks that the worktree is clean, creates an unlinked
Git bundle, and passes its file descriptor over the private publisher channel;
the bundle is never written into a worktree-visible path:

```bash
if [ "${GC_PUBLISH_ENABLED:-0}" = 1 ] \
  && [ "${GC_PUBLISH_OPEN_PR:-0}" = 1 ]; then
  PUBLICATION_JSON=$(python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/publish-pr.py" request \
    --socket "$GC_PUBLISHER_CHANNEL_SOCKET" \
    --worktree "$GC_WORKTREE" \
    --worktree-id "$GC_WORKTREE_ID" \
    --run-id "$GC_RUN_ID" \
    --bead-id "$GC_ROOT_BEAD_ID" \
    --repository "$GC_REPOSITORY" \
    --base "$GC_BASE_BRANCH" \
    --head "$GC_HEAD_BRANCH" \
    --title "$GC_PR_TITLE" \
    --body "$GC_PR_BODY" \
    --installation-id "$GC_GITHUB_INSTALLATION_ID" \
    --app-id "$GC_GITHUB_APP_ID" \
    --cancellation-root "$GC_CANCEL_ROOT" \
    --no-notify)
  PR_URL=$(printf '%s' "$PUBLICATION_JSON" | jq -r '.publication.pr_url')
  test -n "$PR_URL" && test "$PR_URL" != null
  bd update "$GC_ROOT_BEAD_ID" \
    --set-metadata "pull_request_url=$PR_URL"
  python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" \
    publication-notify \
    --socket "$GC_DISCORD_CHANNEL_SOCKET" \
    --body "Published $PR_URL for run ${GC_RUN_ID}."
fi
```

The publisher imports the bundle into its own bare clone, validates the fixed
repository, installation, base branch, managed `gascity/` namespace, exact
head, and `head != base`, then performs only a non-force exact ref update.
It adopts an exact open or merged pull request, blocks a closed-unmerged,
multiple, cross-repository, or divergent match, and creates a pull request
only when absent.  The returned pull-request URL is stored on the root bead
before the Discord notification is sent.  No merge or auto-merge operation is
permitted.
