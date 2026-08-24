# Operations and recovery

R10 diagnostics use structured `tracing` spans/events at command admission and
latch, runtime frame boundaries, GPU pass/readback, audio and I/O delivery, and
distribution queues. The engine metrics and flight recorder expose:

- deadline slack/misses and Program drop/repeat counters;
- source video/audio skew and A/V drift;
- GPU pass/readback last and maximum duration plus device loss;
- command/distribution queue depth and high-water marks;
- audio xrun, underflow, overflow, and device error state;
- persistence failures, including autosave/journal write errors.

The in-memory flight recorder retains at most 45 seconds and 16,384 redacted
events. Both limits are enforced. Keys and embedded text that can contain
tokens, passwords, authorization values, secrets, or native handles are
redacted before entering the recorder. Export uses a temporary file, file sync,
rename, and (where supported) directory sync.

Desktop writes `eiviz-flight-recorder.json`,
`eiviz-capabilities.json`, and `eiviz-crash-report.json` only when requested.
The panic hook also writes the crash report path selected by
`EIVIZ_CRASH_REPORT_PATH` (default `eiviz-crash-report.json`). Crash reports
contain the project hash and recent redacted diagnostics, not the project,
credentials, or native handles.

At startup, a divergent autosave opens an explicit Recover/Discard prompt.
Until the operator decides, Desktop neither loads nor overwrites that autosave.
Recovery changes only the in-memory project; replacing `project.json` still
requires an explicit Save. Corrupt autosaves are reported and can only be
discarded.

Capability reports distinguish compiled, currently available, and active
states. Hardware and interoperability entries remain `hil_pending` until
physical evidence exists; capability export is not HIL evidence and never
activates a fallback.

Generate release SBOM evidence with:

```bash
python3 scripts/generate-sbom.py
```

This writes SPDX 2.3 and CycloneDX 1.5 JSON under `target/sbom`. CI uploads both
as the `eiviz-sbom` artifact.
