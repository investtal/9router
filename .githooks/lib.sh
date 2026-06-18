#!/usr/bin/env bash
# Shared helpers for Investtal git hooks (branch-name + commit-msg enforcement).
# Sourced by post-checkout and commit-msg hooks. Pure functions, no side effects
# beyond what the caller invokes. POSIX-flavoured bash (works on macOS bash 3.2).

# Marker used by the installer to recognise previously-injected chain lines.
IVT_HOOK_MARKER="ivt-hooks-chain"

# Branch names that are always allowed regardless of the IVT-XXXX rule.
# Compared with exact match OR prefix-glob ("release/" matches "release/anything").
IVT_ALLOWED_EXACT="main master develop dev"
IVT_ALLOWED_PREFIX="release/ hotfix/"

# Regex for the IVT task id: IVT- followed by EXACTLY 4 digits (0000-9999),
# then a non-alphanumeric boundary (or end-of-string). The boundary is required
# so that IVT-99999, IVT-0999X, and IVT-69696969 are rejected — they don't
# contain a clean 4-digit id delimited on both sides. The trailing boundary
# char is captured into group 1 so the caller can strip it when extracting the
# bare id (see ivt_extract_task_id).
IVT_TASK_REGEX='IVT-[0-9]{4}([^[:alnum:]]|$)'

# Match a 4-digit IVT id anywhere inside a branch name. Same boundary rule.
# Examples that MATCH:   "IVT-0999", "feat/IVT-0999-broker", "IVT-0999-broker"
# Examples that DON'T:   "IVT-999", "IVT-99999", "IVT-0999X", "IVT-69696969"
IVT_TASK_CONTAINED_REGEX='IVT-[0-9]{4}([^[:alnum:]]|$)'

ivt_hook_debug() {
  # Honoured by every hook. Set IVT_HOOK_DEBUG=1 to trace.
  if [ -n "${IVT_HOOK_DEBUG:-}" ]; then
    echo "[ivt-hook] $*" >&2
  fi
}

# Returns the current branch name, or empty string in detached HEAD.
ivt_current_branch() {
  git symbolic-ref --quiet --short HEAD 2>/dev/null
}

# Returns 0 if the given branch name is allowed (exact infra name or prefix glob).
ivt_branch_is_infra() {
  local branch="$1"
  [ -z "$branch" ] && return 1

  local allowed
  for allowed in $IVT_ALLOWED_EXACT; do
    [ "$branch" = "$allowed" ] && return 0
  done

  local prefix
  for prefix in $IVT_ALLOWED_PREFIX; do
    case "$branch" in
      ${prefix}*) return 0 ;;
    esac
  done

  return 1
}

# Returns 0 if the branch name is valid per the Investtal policy:
#   - infra branch (main/master/develop/release/*/hotfix/*), OR
#   - contains an IVT-XXXX token somewhere in the name.
ivt_branch_is_valid() {
  local branch="$1"
  [ -z "$branch" ] && return 1   # detached HEAD is treated as valid (no name)

  if ivt_branch_is_infra "$branch"; then
    return 0
  fi

  # shellcheck disable=SC2076
  if [[ "$branch" =~ $IVT_TASK_CONTAINED_REGEX ]]; then
    return 0
  fi

  return 1
}

# Extract the bare 4-digit task id ("IVT-0999") from a string, or print empty.
# Strips the trailing boundary char captured by IVT_TASK_REGEX. The branch-name
# validator only needs a yes/no answer, but commit-msg needs the literal id.
ivt_extract_task_id() {
  local s="$1"
  local match
  if [[ "$s" =~ $IVT_TASK_REGEX ]]; then
    match="${BASH_REMATCH[0]}"
    # Drop the trailing boundary character (anything not [0-9], or nothing if
    # the match ended at EOL where BASH_REMATCH[1] is empty).
    printf '%s' "${match%${BASH_REMATCH[1]}}"
  fi
}

# Print a friendly, copy-pasteable explanation of the policy.
ivt_print_policy() {
  cat >&2 <<'EOF'

[branch policy] Investtal task-id branch naming is enforced.

  Allowed branch names must be EITHER:
    • an infra branch: main, master, develop, dev, release/*, hotfix/*
    • contain a task id of the form IVT-XXXX (4 digits), e.g.
        IVT-0999
        feat/IVT-0999-broker-view
        IVT-0999-broker-view

  Examples of INVALID names:
    feat/broker-view          (no task id)
    chore/cleanup             (no task id)
    IVT-999                   (only 3 digits — must be 4)
    IVT-0999X                 (extra chars after the 4 digits)
    IVT-69696969              (more than 4 digits)

To create a correctly-named branch from your current one:
    git branch -m IVT-XXXX-short-description
EOF
}

# Resolve a (possibly symlinked) path to its absolute, dereferenced target.
# Portable across macOS bash 3.2 (no readlink -f on stock < 12.3), Linux
# (GNU readlink -f), and systems with coreutils (greadlink). Returns the
# original path if it isn't a symlink.
ivt_resolve_symlink() {
  local target="$1"

  # Prefer GNU/coreutils readlink -f when available (handles arbitrary depth).
  if command -v greadlink >/dev/null 2>&1; then
    greadlink -f "$target" 2>/dev/null && return
  fi
  # Stock readlink -f works on Linux and macOS 12.3+. Probe with a known path
  # to avoid emitting an error on BSD readlink (which lacks -f and would print
  # a usage message to stdout).
  if command -v readlink >/dev/null 2>&1 \
     && readlink -f -- / >/dev/null 2>&1; then
    readlink -f -- "$target" 2>/dev/null && return
  fi

  # Manual fallback: chase symlinks one hop at a time. Bash 3.2-safe.
  local cur="$target"
  local hops=0
  while [ -L "$cur" ] && [ "$hops" -lt 40 ]; do
    hops=$((hops + 1))
    local dir
    dir="$(cd -P "$(dirname "$cur")" >/dev/null 2>&1 && pwd)"
    local link
    link="$(readlink "$cur")"
    case "$link" in
      /*) cur="$link" ;;
      *)  cur="$dir/$link" ;;
    esac
  done
  # Canonicalise the directory (the leaf need not exist for our callers).
  local d
  d="$(cd -P "$(dirname "$cur")" >/dev/null 2>&1 && pwd)" || { printf '%s\n' "$cur"; return; }
  printf '%s/%s\n' "$d" "$(basename "$cur")"
}

# Return the absolute directory the currently-running hook script lives in,
# dereferencing symlinks (the installer symlinks our hooks into each repo, so
# $0 is the symlink but lib.sh sits next to the real file).
ivt_hook_dir() {
  ivt_resolve_symlink "$0" | { read -r resolved; dirname "$resolved"; }
}

# Chain-execute a sibling hook that was NOT installed by ivt-hooks.
# Pass the hook name (e.g. "pre-commit"); the function locates the next
# non-ivt hook file SIBLING to this running hook and runs it with the
# original args. Sibling (not git-path) resolution matters because the
# installer places .pre-ivt files next to our hook, which in husky/vite-hooks
# repos is NOT .git/hooks. Recursion safe via IVT_HOOK_CHAIN_DEPTH.
ivt_chain_next_hook() {
  local hook_name="$1"
  shift

  # Stop runaway recursion explicitly.
  local depth="${IVT_HOOK_CHAIN_DEPTH:-0}"
  if [ "$depth" -gt 8 ]; then
    ivt_hook_debug "chain depth limit hit at $hook_name, stopping"
    return 0
  fi

  local hooks_dir
  hooks_dir="$(ivt_hook_dir)"
  [ -z "$hooks_dir" ] && return 0

  # The installer renames any pre-existing hook to "<name>.pre-ivt" and chains it.
  local previous="$hooks_dir/${hook_name}.pre-ivt"
  if [ -x "$previous" ]; then
    ivt_hook_debug "chaining $previous (depth=$((depth + 1)))"
    IVT_HOOK_CHAIN_DEPTH=$((depth + 1)) "$previous" "$@"
    return $?
  fi

  # Legacy fallback: run a sibling "<hook_name>.next" if present.
  local legacy="$hooks_dir/${hook_name}.next"
  if [ -x "$legacy" ]; then
    ivt_hook_debug "chaining (legacy) $legacy"
    IVT_HOOK_CHAIN_DEPTH=$((depth + 1)) "$legacy" "$@"
    return $?
  fi

  return 0
}
