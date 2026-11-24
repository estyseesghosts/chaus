# Diagnostics

Release GUI builds write a small local event log. It exists to answer what
phase a launch or save reached; it is not a telemetry service and nothing is
uploaded.

## Location and retention

- Windows: `%LOCALAPPDATA%\Chaus\Diagnostics\chaus.log`
- Linux: `$XDG_STATE_HOME/chaus/diagnostics/chaus.log`, or
  `~/.local/state/chaus/diagnostics/chaus.log` when `XDG_STATE_HOME` is unset.

When the active log exceeds 256 KiB it is renamed to `chaus.1.log` and a new
`chaus.log` starts. Only those two files are kept. Set
`CHAUS_DIAGNOSTICS_DIR` to move the sink, or delete both files to clear it. A
machine with no writable state directory simply runs without diagnostics.

Each line carries a sequence number, timestamp, build SHA, correlation id,
phase and outcome. Phases are `startup`, `shutdown` and `panic`; the correlation
id ties future per-window and per-save events together. Messages are redacted:
path-like tokens become `<path>`, panics keep only the source file name, and
each message is capped at 240 characters. Document text, clipboard, keystrokes
and recovery payloads are never written.

## Reading a Windows GUI crash

A panic record identifies a Rust unwind. A native access violation or abort has
no Rust panic stack; use the OS record instead:

1. Open Event Viewer → Windows Logs → Application.
2. Look for an `Application Error` entry at the crash time. Record the
   exception code, faulting module name and version, and the process id.
3. Match the `build=` field in `chaus.log` to the source SHA. If the SHA is
   `unknown`, the binary was built outside a git checkout.
4. Windows Error Reporting may also have a local minidump under
   `%LOCALAPPDATA%\CrashDumps`. Copy it only if the user agrees; it can contain
   process memory.

Do not treat "no panic record" as evidence that no crash happened; a native
fault or a process killed by the OS produces no Rust panic event.

## Failure-mode triage

| Symptom | What to record |
| --- | --- |
| Window never appears | Last logged phase, build SHA, OS version |
| Save reports failure | Save phase outcome and the editor status text; the log holds no file text |
| Process exits while saving | Event Viewer exception code and faulting module, build SHA, last phase |
| Recovery unavailable | The visible editor status; recovery failures are NOT logged with content |

Integration tests in `crates/chaus-app/tests/diagnostics.rs` exercise the sink,
rotation, redaction and the panic hook without a GUI.
