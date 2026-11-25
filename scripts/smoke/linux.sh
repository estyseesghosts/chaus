#!/bin/bash
#
# Native Linux smoke test for a packaged chaus executable.
#
# Copies the binary and a fixture into a temporary work directory, launches
# that copy with a temporary HOME, and drives the real Silvestris OpenPanel
# and SavePanel through X11. No panel internals are called and no native
# picker is involved. The named startup document makes both panels begin in
# the temporary work directory.
#
# Requires an interactive X11 session plus xdotool, wmctrl and xclip. A
# Wayland-only session needs an XWayland-provided DISPLAY and is not an
# equivalent native Wayland acceptance run. This script controls the desktop;
# run it only on a dedicated machine or interactive runner.

set -euo pipefail

err() {
  echo "[$(date +'%Y-%m-%dT%H:%M:%S%z')]: $*" >&2
}

die() {
  err "$*"
  exit 1
}

require_tool() {
  local tool="$1"
  command -v "${tool}" >/dev/null 2>&1 \
    || die "missing ${tool}; install it on the interactive GUI runner"
}

usage() {
  cat <<'END'
Usage: linux.sh --exe PATH [--report-dir DIR] [--skip-panels] [--screenshots]

Drives a packaged chaus binary through the Silvestris file panels on X11.
END
}

EXE=""
REPORT_DIR=""
SKIP_PANELS=0
SCREENSHOTS=0

while (($#)); do
  case "$1" in
    --exe)
      EXE="${2:-}"
      shift 2
      ;;
    --report-dir)
      REPORT_DIR="${2:-}"
      shift 2
      ;;
    --skip-panels)
      SKIP_PANELS=1
      shift
      ;;
    --screenshots)
      SCREENSHOTS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "${EXE}" ]] || die "--exe is required"
[[ -f "${EXE}" ]] || die "executable not found: ${EXE}"
[[ -n "${DISPLAY:-}" ]] || die "DISPLAY is not set; run inside an interactive X11 session"

for tool in xdotool wmctrl xclip import; do
  if (( SCREENSHOTS )) || [[ "${tool}" != "import" ]]; then
    require_tool "${tool}"
  fi
done

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/chaus-smoke-XXXXXX")"
APP_DIR="${ROOT}/app"
WORK_DIR="${ROOT}/work"
PROFILE_DIR="${ROOT}/profile"
REPORT_DIR="${REPORT_DIR:-${ROOT}/report}"
mkdir -p "${APP_DIR}" "${WORK_DIR}" "${PROFILE_DIR}" "${REPORT_DIR}"
cp -- "${EXE}" "${APP_DIR}/chaus"
chmod +x "${APP_DIR}/chaus"
APP="${APP_DIR}/chaus"
PID=""

STEPS=()
FAILURE=""

step() {
  STEPS+=("$1: $2")
  echo "[ok] $1 - $2"
}

cleanup() {
  if [[ -n "${PID}" ]] && kill -0 "${PID}" 2>/dev/null; then
    kill "${PID}" 2>/dev/null || true
    wait "${PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

write_report() {
  local report="${REPORT_DIR}/smoke-report.json"
  local steps_json="" entry escaped
  for entry in "${STEPS[@]}"; do
    escaped="${entry//\\/\\\\}"
    escaped="${escaped//\"/\\\"}"
    if [[ -n "${steps_json}" ]]; then
      steps_json+=","
    fi
    steps_json+="\"${escaped}\""
  done
  {
    printf '{\n'
    printf '  "platform": "linux",\n'
    printf '  "executable": "%s",\n' "${APP}"
    printf '  "panels": "Silvestris OpenPanel/SavePanel (pinned f1c2a010)",\n'
    printf '  "skipped_panels": %s,\n' "$(( SKIP_PANELS ))"
    printf '  "steps": [%s],\n' "${steps_json}"
    printf '  "failure": "%s",\n' "${FAILURE//\"/\\\"}"
    printf '  "root": "%s"\n' "${ROOT}"
    printf '}\n'
  } >"${report}"
  echo "report: ${report}"
}

send_keys() {
  xdotool key --clearmodifiers "$@"
  sleep 0.3
}

type_text() {
  xdotool type --clearmodifiers --delay 30 -- "$1"
  sleep 0.3
}

paste_text() {
  printf '%s' "$1" | xclip -selection clipboard
  send_keys ctrl+a
  send_keys ctrl+v
}

wait_for_window() {
  local title="$1" seconds="${2:-15}" deadline end
  deadline=$(( SECONDS + seconds ))
  while (( SECONDS < deadline )); do
    if wmctrl -lp | grep -F -- "${title}" | grep -q -- " ${PID} "; then
      return 0
    fi
    sleep 0.2
  done
  err "window '${title}' did not appear (windows: $(wmctrl -lp | grep -F -- "${PID}" || true))"
  return 1
}

wait_for_window_gone() {
  local title="$1" seconds="${2:-8}" deadline
  deadline=$(( SECONDS + seconds ))
  while (( SECONDS < deadline )); do
    if ! wmctrl -lp | grep -F -- "${title}" | grep -q -- " ${PID} "; then
      return 0
    fi
    sleep 0.2
  done
  err "window '${title}' did not close"
  return 1
}

focus_window() {
  local title="$1" id
  id="$(xdotool search --onlyvisible --pid "${PID}" --name "${title}" | head -n 1)"
  [[ -n "${id}" ]] || return 1
  xdotool windowactivate --sync "${id}"
  sleep 0.4
}

wait_for_file_text() {
  local path="$1" needle="$2" seconds="${3:-10}" deadline
  deadline=$(( SECONDS + seconds ))
  while (( SECONDS < deadline )); do
    if [[ -f "${path}" ]] && grep -q -- "${needle}" "${path}"; then
      return 0
    fi
    sleep 0.15
  done
  err "file ${path} did not reach the expected content"
  return 1
}

write_fixture() {
  local name="$1" text="$2"
  printf '%s' "${text}" >"${WORK_DIR}/${name}"
  printf '%s\n' "${WORK_DIR}/${name}"
}

start_app() {
  local title="$1" argument="${2:-}"
  if [[ -n "${argument}" ]]; then
    (cd "${WORK_DIR}" && HOME="${PROFILE_DIR}" "${APP}" "${argument}") &
  else
    (cd "${WORK_DIR}" && HOME="${PROFILE_DIR}" "${APP}") &
  fi
  PID=$!
  wait_for_window "${title}" 20
  focus_window "${title}" || true
}

save_sheet() {
  local name="$1" use_clipboard="${2:-0}"
  send_keys ctrl+shift+s
  sleep 1.2
  if [[ "${use_clipboard}" == "1" ]]; then
    paste_text "${name}"
  else
    type_text "${name}"
  fi
}

screenshot() {
  if (( SCREENSHOTS )); then
    import -window root "${REPORT_DIR}/$1.png"
  fi
}

close_app() {
  send_keys alt+F4
  for _ in $(seq 1 40); do
    kill -0 "${PID}" 2>/dev/null || return 0
    sleep 0.25
  done
  die "the app did not exit after Alt+F4"
}

run_basic_phases() {
  local marker path
  marker="SMOKE$(date -u +%H%M%S)"
  path="$(write_fixture sample.txt 'titlebar smoke')"
  start_app "sample.txt" "${path}"
  send_keys ctrl+End
  type_text "${marker}"
  send_keys ctrl+s
  wait_for_file_text "${path}" "${marker}"
  step "typing-save" "typed and saved ${marker}"
  send_keys ctrl+a ctrl+c ctrl+End Return ctrl+v ctrl+s
  wait_for_file_text "${path}" "SMOKE"
  step "clipboard" "copy/paste worked in the packaged window"

  local vim_path
  vim_path="$(write_fixture vim.txt $'first line\nSECOND_MARKER\n')"
  close_app
  start_app "vim.txt" "${vim_path}"
  send_keys ctrl+shift+v
  sleep 0.4
  type_text "ggdd"
  send_keys ctrl+s
  wait_for_file_text "${vim_path}" "SECOND_MARKER"
  if grep -q "first line" "${vim_path}"; then
    die "Vim dd removed the wrong line"
  fi
  step "vim" "Ctrl+Shift+V then dd removed the first line"
  close_app
}

run_panel_phases() {
  local base_name open_name save_name cancel_name replace_name keep_name
  local unicode_name unicode_path replace_path keep_path base_path
  base_name="panel-base.txt"
  open_name="open-me.txt"
  save_name="saved-as.txt"
  cancel_name="cancel-as.txt"
  replace_name="replace-target.txt"
  keep_name="keep-target.txt"
  unicode_name=$'n\xc3\xb6te-\xe6\x96\x87\xe4\xbb\xb6.txt'

  base_path="$(write_fixture "${base_name}" 'panel starting document')"
  write_fixture "${open_name}" 'opened through the browser'
  replace_path="$(write_fixture "${replace_name}" 'old contents')"
  keep_path="$(write_fixture "${keep_name}" 'keep me')"
  unicode_path="${WORK_DIR}/${unicode_name}"
  start_app "${base_name}" "${base_path}"

  send_keys ctrl+o
  wait_for_window "Chaus: Open" 15
  step "open-panel" "Ctrl+O presented the Silvestris browser at the work directory"
  screenshot "open-panel"
  send_keys Tab Tab
  type_text "${open_name}"
  send_keys Return
  wait_for_window "${open_name}" 15
  step "open-accept" "selected ${open_name} in the browser and accepted with Return"
  wmctrl -lp | grep -F -- "${base_name}" | grep -q -- " ${PID} " \
    || die "opening a document replaced the original window"

  focus_window "${open_name}" || die "cannot focus the opened document"
  send_keys ctrl+o
  wait_for_window "Chaus: Open" 15
  send_keys Escape
  wait_for_window_gone "Chaus: Open"
  step "open-cancel" "Escape dismissed the browser and opened nothing"

  save_sheet "${save_name}"
  send_keys Return
  wait_for_file_text "${WORK_DIR}/${save_name}" "opened through the browser"
  wait_for_window "${save_name}" 15
  step "save-as-accept" "typed only ${save_name}; the panel resolved it in the work directory"

  save_sheet "${cancel_name}"
  send_keys Escape
  sleep 0.7
  [[ ! -e "${WORK_DIR}/${cancel_name}" ]] \
    || die "a cancelled Save As wrote a file"
  step "save-as-cancel" "Escape dismissed the Save sheet without writing"

  save_sheet "${replace_name}"
  send_keys Return
  wait_for_window "Replace" 15
  send_keys Return
  wait_for_file_text "${replace_path}" "opened through the browser"
  wait_for_window "${replace_name}" 15
  step "replace-existing" "Replace confirmed the collision and wrote the existing target"

  save_sheet "${keep_name}"
  send_keys Return
  wait_for_window "Replace" 15
  send_keys Escape
  sleep 0.5
  send_keys Escape
  sleep 0.7
  grep -q "keep me" "${keep_path}" || die "cancelling Replace overwrote the target"
  step "replace-cancel" "Escape on the alert returned to the sheet; Escape kept the target"

  save_sheet "${unicode_name}" 1
  send_keys Return
  wait_for_file_text "${unicode_path}" "opened through the browser"
  wait_for_window "${unicode_name}" 15
  step "unicode-save-as" "a pasted Unicode name round-tripped through the Name field"

  close_app
}

main() {
  run_basic_phases
  if (( SKIP_PANELS == 0 )); then
    run_panel_phases
  fi
}

if main; then
  write_report
  exit 0
else
  FAILURE="native smoke phase failed; see console output"
  write_report
  exit 1
fi
