#!/bin/sh
# Check the app's crates for the iPad (aarch64-apple-ios), one by one, and say which build and which do not
# (#41). Nothing ships for the iPad yet: this shows what stands in the way, and keeps what builds building.
#
# A crate in MUST, once it builds, failing fails the check; the others are only reported. Writes a table
# to $GITHUB_STEP_SUMMARY when there is one. Needs a Mac with Xcode and the aarch64-apple-ios Rust target.
set -u

target=aarch64-apple-ios
crates="alcyo-board alcyo-models alcyo-fonts alcyo-meetings alcyo-export alcyo-search alcyo-share alcyo-keep alcyo-update alcyo-import alcyo-llama alcyo-agent alcyo-audio alcyo-ui"
# Those that build for the iPad today, kept that way.
MUST=${MUST:-}

summary=${GITHUB_STEP_SUMMARY:-/dev/stdout}
logs=$(mktemp -d)
failed_must=""
{
  echo "### iPad ($target)"
  echo
  echo "| crate | builds | first error |"
  echo "| --- | --- | --- |"
} >> "$summary"
for crate in $crates; do
  if cargo check --target "$target" -p "$crate" > "$logs/$crate.log" 2>&1; then
    echo "| $crate | yes | |" >> "$summary"
    echo "$crate: builds"
  else
    first=$(grep -m1 -E '^error|error:|failed to run custom build command' "$logs/$crate.log" | cut -c1-160 | tr '|' '/')
    echo "| $crate | no | \`${first:-see the log}\` |" >> "$summary"
    echo "$crate: does not build"
    echo "::group::$crate"
    tail -40 "$logs/$crate.log"
    echo "::endgroup::"
    case " $MUST " in *" $crate "*) failed_must="$failed_must $crate" ;; esac
  fi
done
if [ -n "$failed_must" ]; then
  echo "built for the iPad before, and no longer:$failed_must" >&2
  exit 1
fi
