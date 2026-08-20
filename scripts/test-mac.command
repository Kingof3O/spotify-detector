#!/bin/bash

set -u

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This launcher is for macOS only."
  exit 1
fi

project_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_dir" || exit 1

find_cargo() {
  local candidate
  local mac_user
  local architecture

  if command -v cargo >/dev/null 2>&1; then
    command -v cargo
    return 0
  fi

  mac_user="$(id -un)"
  case "$(uname -m)" in
    arm64) architecture="aarch64" ;;
    x86_64) architecture="x86_64" ;;
    *) architecture="$(uname -m)" ;;
  esac

  for candidate in \
    "/Users/${mac_user}/.cargo/bin/cargo" \
    "/Users/${mac_user}/.rustup/toolchains/stable-${architecture}-apple-darwin/bin/cargo" \
    "/opt/homebrew/bin/cargo" \
    "/usr/local/bin/cargo"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  return 1
}

cargo_command="$(find_cargo || true)"
if [[ -z "$cargo_command" ]]; then
  echo "Rust/Cargo could not be found."
  echo "Install Rust from https://rustup.rs, then run this file again."
  exit 1
fi

cargo_bin_dir="$(dirname "$cargo_command")"
export PATH="$cargo_bin_dir:$PATH"

base_url="http://127.0.0.1:${SPOTIFY_OVERLAY_PORT:-18923}"
test_url="${SPOTIFY_OVERLAY_TEST_URL:-${base_url}/test?long=1}"
log_file="$(mktemp -t spotify-overlay-mac-test.XXXXXX)"
app_pid=""

stop_started_app() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -f "$log_file"
}

trap stop_started_app INT TERM EXIT

if curl -fsS --max-time 1 "$base_url/health" >/dev/null 2>&1; then
  echo "An overlay server is already running at $base_url."
  echo "Opening the Mac visual test: $test_url"
  open "$test_url"
  trap - INT TERM EXIT
  exit 0
fi

echo "Starting the local overlay test server..."
"$cargo_command" run --quiet >"$log_file" 2>&1 &
app_pid=$!

healthy=0
for attempt in {1..40}; do
  if curl -fsS --max-time 1 "$base_url/health" >/dev/null 2>&1; then
    healthy=1
    break
  fi

  if ! kill -0 "$app_pid" 2>/dev/null; then
    break
  fi
  sleep 0.25
done

if [[ "$healthy" -ne 1 ]]; then
  echo "The overlay test server did not start."
  echo
  sed -n '1,120p' "$log_file"
  echo
  echo "If Rust reports that no toolchain is configured, run:"
  echo "  rustup default stable"
  exit 1
fi

echo "Opening the Mac visual test: $test_url"
if ! open "$test_url"; then
  echo "Could not open the browser automatically. Open this URL manually:"
  echo "  $test_url"
fi

echo
echo "Test page is running. Resize the browser to 650 x 250 to match OBS."
echo "The default long title checks the seamless marquee loop."
echo "Press Ctrl+C in this window to stop the test server."
echo

wait "$app_pid"
