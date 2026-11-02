#!/usr/bin/env bash
# The steps of the driftmark GitHub Action (action.yml), one per subcommand:
#
#   key      say where a build from source is cached, and under which key
#   install  build driftmark from source, download a release, or use the one
#            on PATH
#   check    lint the changed Markdown files as annotations, and export the
#            rendered changes as one HTML file
#   comment  write the summary comment to the job summary, and post it on the
#            pull request or update the one posted before
#   finish   fail the job when `lint: fail` and the lint found an error
#
# Every value comes from the environment, never from text pasted into the
# script, so nothing in a branch name or a folder can run as a command.
# docs/features/ci.md describes the action; this script needs bash, git, curl
# and jq, which every GitHub-hosted runner has.

set -euo pipefail

# The oldest Rust that builds driftmark (Cargo.toml, rust-version).
MIN_RUST_MINOR=95

die() {
  echo "::error title=driftmark::$*"
  exit 1
}

# Appends `name=value` to the step's outputs.
output() {
  echo "$1=$2" >>"${GITHUB_OUTPUT:?the runner sets GITHUB_OUTPUT}"
}

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

# `.exe` on Windows, nothing elsewhere.
exe() {
  case "${RUNNER_OS:-}" in Windows) echo .exe ;; *) echo "" ;; esac
}

# The repository this action was downloaded from: its root holds Cargo.toml.
source_root() {
  (cd "${GITHUB_ACTION_PATH:?the runner sets GITHUB_ACTION_PATH}/../../.." && pwd)
}

step_key() {
  case "${DRIFTMARK_VERSION:-source}" in
    source)
      local root hash dir
      root=$(source_root)
      [ -f "$root/Cargo.toml" ] || die "no Cargo.toml at $root to build driftmark from"
      # Every file the build reads: the manifests, the lock file and the
      # crates. The same sources give the same key, on any runner.
      hash=$(cd "$root" && find Cargo.toml Cargo.lock crates -type f -print0 |
        LC_ALL=C sort -z | xargs -0 cat | sha256 | cut -c1-20)
      dir="${RUNNER_TOOL_CACHE:-$HOME/.cache}/driftmark/source-$hash"
      output key "driftmark-${RUNNER_OS:-os}-${RUNNER_ARCH:-arch}-source-$hash"
      output dir "$dir"
      ;;
    *)
      output key ""
      output dir ""
      ;;
  esac
}

# Builds driftmark from the action's repository into $1. It is a dev build,
# as driftmark's own CI makes: no debug info and no incremental files,
# because a hosted runner has about 14 GB free. It compiles about three
# times faster than a release build, and a merge request's range still takes
# well under a second. Symbols are stripped to keep the cache small, and
# debug assertions and overflow checks are off, so it behaves as a release
# build does.
build() {
  local dir=$1 root target minor
  root=$(source_root)
  command -v cargo >/dev/null ||
    die "cargo is not installed; add a Rust toolchain step before this action, or use a release with version: latest"
  minor=$(rustc --version | sed -E 's/^rustc 1\.([0-9]+).*/\1/')
  if [ "$minor" -lt "$MIN_RUST_MINOR" ]; then
    command -v rustup >/dev/null || die "driftmark needs Rust 1.$MIN_RUST_MINOR or newer, and this runner has $(rustc --version)"
    rustup toolchain install stable --profile minimal --no-self-update
    export RUSTUP_TOOLCHAIN=stable
  fi
  target="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/driftmark-target"
  echo "Building driftmark from $root; the next run with these sources takes it from the cache."
  CARGO_TARGET_DIR="$target" \
    CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_INCREMENTAL=0 \
    CARGO_PROFILE_DEV_STRIP=symbols \
    CARGO_PROFILE_DEV_DEBUG_ASSERTIONS=false \
    CARGO_PROFILE_DEV_OVERFLOW_CHECKS=false \
    cargo build --locked -p driftmark --manifest-path "$root/Cargo.toml"
  mkdir -p "$dir"
  cp "$target/debug/driftmark$(exe)" "$dir/"
}

# Sets `file` to the release file's name for this runner, as the release
# workflow (M6) names them: driftmark-<target>.tar.gz, or .zip on Windows.
# (Never called as `$(...)`: `die` there would only fail a subshell, and its
# message would land in the variable.)
release_file() {
  local arch
  case "${RUNNER_ARCH:-$(uname -m)}" in
    X64 | x86_64 | amd64) arch=x86_64 ;;
    ARM64 | aarch64 | arm64) arch=aarch64 ;;
    *) die "no driftmark release for the processor ${RUNNER_ARCH:-$(uname -m)}" ;;
  esac
  case "${RUNNER_OS:-$(uname -s)}" in
    Linux) file="driftmark-$arch-unknown-linux-gnu.tar.gz" ;;
    macOS | Darwin) file="driftmark-$arch-apple-darwin.tar.gz" ;;
    Windows) file="driftmark-$arch-pc-windows-msvc.zip" ;;
    *) die "no driftmark release for ${RUNNER_OS:-$(uname -s)}" ;;
  esac
}

# Downloads release $1 (a version such as 0.1.0, or `latest`) into $2,
# checked against the release's SHA256SUMS.
download() {
  local version=$1 dir=$2 file="" base sums
  release_file
  base="${GITHUB_SERVER_URL:-https://github.com}/${DRIFTMARK_REPOSITORY:-cj-ways/driftmark}/releases"
  case "$version" in
    latest) base="$base/latest/download" ;;
    *) base="$base/download/v${version#v}" ;;
  esac
  mkdir -p "$dir"
  echo "Downloading $base/$file"
  curl -fsSL --retry 3 -o "$dir/$file" "$base/$file" || die "could not download $base/$file"
  curl -fsSL --retry 3 -o "$dir/SHA256SUMS" "$base/SHA256SUMS" || die "could not download $base/SHA256SUMS"
  sums=$(grep -E "^[0-9a-f]{64}  \*?$file\$" "$dir/SHA256SUMS" | cut -c1-64) ||
    die "SHA256SUMS names no $file"
  [ "$(sha256 "$dir/$file" | cut -c1-64)" = "$sums" ] || die "$file does not match its SHA256SUMS line"
  case "$file" in
    *.zip) (cd "$dir" && unzip -oq "$file") ;;
    *) tar -xzf "$dir/$file" -C "$dir" ;;
  esac
  rm -f "$dir/$file" "$dir/SHA256SUMS"
}

step_install() {
  local version=${DRIFTMARK_VERSION:-source} dir
  case "$version" in
    installed)
      command -v driftmark >/dev/null || die "version: installed, but driftmark is not on PATH"
      dir=$(dirname "$(command -v driftmark)")
      ;;
    source)
      dir=${DRIFTMARK_DIR:?the key step sets the cache folder}
      if [ -x "$dir/driftmark$(exe)" ]; then
        echo "driftmark is cached for these sources."
      else
        build "$dir"
      fi
      ;;
    *)
      dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/driftmark-$version"
      download "$version" "$dir"
      ;;
  esac
  "$dir/driftmark$(exe)" --version
  echo "$dir" >>"${GITHUB_PATH:?the runner sets GITHUB_PATH}"
}

# The folders input, one word per folder, in the array `folders`. A folder
# name cannot hold a space; `*` stays a `*`, never a list of files.
read_folders() {
  folders=()
  local word
  set -f
  for word in ${DRIFTMARK_FOLDERS:-}; do folders+=("$word"); done
  set +f
}

# Sets `range`, which both commands read: what the head added since it left
# the base.
set_range() {
  [ -n "${DRIFTMARK_BASE:-}" ] && [ -n "${DRIFTMARK_HEAD:-}" ] ||
    die "no base or head commit: run on a pull_request event, or give the base and head inputs"
  range="$DRIFTMARK_BASE...$DRIFTMARK_HEAD"
}

step_check() {
  command -v jq >/dev/null || die "jq is not installed"
  cd "${GITHUB_WORKSPACE:?the runner sets GITHUB_WORKSPACE}"
  local range="" documents code=0 file
  set_range
  for commit in "${DRIFTMARK_BASE}" "${DRIFTMARK_HEAD}"; do
    git cat-file -e "$commit^{commit}" 2>/dev/null ||
      die "commit $commit is not in this clone; check out with fetch-depth: 0"
  done
  read_folders
  documents=$(driftmark diff "$range" --json=summary -- ${folders[@]+"${folders[@]}"} |
    jq '.documents | length') || die "driftmark diff could not compare $range; its message is above"
  output documents "$documents"
  echo "driftmark: Markdown documents changed in $range: $documents."
  case "${DRIFTMARK_LINT:-report}" in
    off) ;;
    report | fail)
      driftmark lint --format github --changed "$range" ${folders[@]+"${folders[@]}"} || code=$?
      case "$code" in
        0) output lint-errors false ;;
        1) output lint-errors true ;;
        *) die "driftmark lint could not run (exit code $code)" ;;
      esac
      ;;
    *) die "lint takes report, fail or off, not ${DRIFTMARK_LINT}" ;;
  esac
  if [ "$documents" -gt 0 ]; then
    mkdir -p "${RUNNER_TEMP:?the runner sets RUNNER_TEMP}/driftmark"
    file="$RUNNER_TEMP/driftmark/driftmark-changes-${DRIFTMARK_HEAD:0:12}.html"
    driftmark export changes "$range" -o "$file" -- ${folders[@]+"${folders[@]}"}
    output export "$file"
  else
    output export ""
  fi
}

# Calls the GitHub API: method, path, and an optional JSON body file. The
# response body goes to the file `response`; the HTTP status is printed.
api() {
  local method=$1 path=$2 body=${3:-}
  local args=(-sS -X "$method" -o response -w '%{http_code}'
    -H "Authorization: Bearer ${GITHUB_TOKEN:?no token}"
    -H "Accept: application/vnd.github+json"
    -H "X-GitHub-Api-Version: 2022-11-28")
  if [ -n "$body" ]; then
    args+=(-H "Content-Type: application/json" --data-binary "@$body")
  fi
  # A connection that fails prints 000 as its status.
  curl "${args[@]}" "${GITHUB_API_URL:-https://api.github.com}$path" || true
}

# Sets `found` to the id of the comment this action posted before on the
# pull request, or to nothing: the first comment by the token's bot whose
# body starts with the marker. Other people's comments are never edited, even
# with the marker. Sets `status` to the HTTP status of the last page read,
# and fails when a page could not be read.
find_comment() {
  local pr=$1 marker=$2 page=1
  found=""
  while :; do
    status=$(api GET "/repos/$GITHUB_REPOSITORY/issues/$pr/comments?per_page=100&page=$page")
    [ "$status" = 200 ] || return 1
    found=$(jq -r --arg marker "$marker" --arg author "${DRIFTMARK_COMMENT_AUTHOR:-github-actions[bot]}" \
      'map(select(.user.login == $author and (.body | startswith($marker)))) | .[0].id // empty' response)
    [ -z "$found" ] || return 0
    [ "$(jq length response)" -eq 100 ] || return 0
    page=$((page + 1))
  done
}

step_comment() {
  command -v jq >/dev/null || die "jq is not installed"
  cd "${GITHUB_WORKSPACE:?the runner sets GITHUB_WORKSPACE}"
  local range="" body marker link=() pr=${DRIFTMARK_PR:-} found="" status=""
  set_range
  read_folders
  body="${RUNNER_TEMP:?the runner sets RUNNER_TEMP}/driftmark-comment.md"
  if [ -n "${DRIFTMARK_LINK:-}" ]; then link=(--link "$DRIFTMARK_LINK"); fi
  driftmark diff "$range" --format markdown-summary ${link[@]+"${link[@]}"} \
    -- ${folders[@]+"${folders[@]}"} >"$body"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$body" >>"$GITHUB_STEP_SUMMARY"; fi
  output comment-url ""
  [ "${DRIFTMARK_COMMENT:-true}" = true ] || return 0
  if [ -z "$pr" ]; then
    echo "::notice title=driftmark::not a pull request, so no comment; the job summary has it"
    return 0
  fi
  marker=$(head -n 1 "$body")
  cd "$RUNNER_TEMP"
  if ! find_comment "$pr" "$marker"; then
    cannot_comment "$status"
    return 0
  fi
  if [ -z "$found" ] && [ "${DRIFTMARK_DOCUMENTS:-0}" = 0 ]; then
    echo "No Markdown document changed and no comment to update."
    return 0
  fi
  jq -Rs '{body: .}' <"$body" >payload.json
  if [ -n "$found" ]; then
    status=$(api PATCH "/repos/$GITHUB_REPOSITORY/issues/comments/$found" payload.json)
    [ "$status" = 200 ] || { cannot_comment "$status"; return 0; }
    echo "Updated comment $found."
  else
    status=$(api POST "/repos/$GITHUB_REPOSITORY/issues/$pr/comments" payload.json)
    [ "$status" = 201 ] || { cannot_comment "$status"; return 0; }
    echo "Posted comment $(jq -r .id response)."
  fi
  output comment-url "$(jq -r .html_url response)"
}

# A comment that could not be written is a warning, not a failed job: the
# job summary still has it. A pull request from a fork gets a read-only
# token, which is the usual reason.
cannot_comment() {
  local message
  message=$(jq -r '.message // empty' response 2>/dev/null || true)
  echo "::warning title=driftmark::could not comment (HTTP $1${message:+: $message}); a pull request from a fork gets a read-only token, and the job needs permissions: pull-requests: write. The job summary has the summary."
}

step_finish() {
  if [ "${DRIFTMARK_LINT:-report}" = fail ] && [ "${DRIFTMARK_LINT_ERRORS:-false}" = true ]; then
    die "driftmark lint found errors in the changed documents; see the annotations"
  fi
}

case "${1:-}" in
  key) step_key ;;
  install) step_install ;;
  check) step_check ;;
  comment) step_comment ;;
  finish) step_finish ;;
  *)
    echo "usage: driftmark.sh key|install|check|comment|finish" >&2
    exit 2
    ;;
esac
