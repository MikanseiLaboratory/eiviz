# 要件トレーサビリティ

このファイルは `cargo run -p eiviz-certification -- matrix` で生成します。`pending` は未実施であり、simulation の成功を HIL 合格として扱いません。

| Requirement | Automated tests | HIL scenarios | HIL status | Artifact paths |
| --- | --- | --- | --- | --- |
| R01 | eiviz-project::tests<br>packaging/tests | FILE-HIL-01 | pending | target/certification/project |
| R02 | eiviz-core::project::tests<br>CERT-MAX-ADMITTED-GRAPH |  | not_applicable | target/certification/evidence.json |
| R03 | eiviz-core::graph::tests<br>CERT-MAX-ADMITTED-GRAPH |  | not_applicable | target/certification/evidence.json |
| R04 | CERT-TIMING-SOAK<br>eiviz-time::tests | TIME-HIL-01..08 | pending | target/certification/evidence.json<br>target/certification/hil/timing |
| R05 | CERT-MAX-ADMITTED-GRAPH<br>eiviz-runtime::tests | GPU-HIL-01..08 | pending | target/certification/evidence.json<br>target/certification/hil/gpu |
| R06 | CERT-TIMING-SOAK<br>eiviz-media::asrc::tests | AUDIO-HIL | pending | target/certification/evidence.json<br>target/certification/hil/audio |
| R07 | CERT-FAULT-NIC-OUTAGE<br>adapter contract tests | NDI-HIL<br>OMT-HIL-01..10<br>DECKLINK-HIL | pending | target/certification/evidence.json<br>target/certification/hil/io |
| R08 | CERT-FAULT-SINK<br>CERT-FAULT-DISK-FULL<br>CERT-FAULT-NIC-OUTAGE | DIST-HIL | pending | target/certification/evidence.json<br>target/certification/hil/distribution |
| R09 | CERT-COMMAND-FLOOD-REPLAY | CONTROL-HIL | pending | target/certification/evidence.json<br>target/certification/hil/control |
| R10 | CERT-TIMING-SOAK<br>CERT-MEMORY-QUEUE-HIGH-WATER | GPU-HIL<br>TIME-HIL<br>AUDIO-HIL | pending | target/certification/evidence.json |
| R11 | CI MSRV/fmt/clippy/test<br>packaging/tests | WINDOWS-RELEASE-HIL | pending | target/certification<br>target/package |
| AC-01 | eiviz-project round_trip | FILE-HIL-01 | pending | target/certification/project |
| AC-02 | eiviz-project migration tests |  | not_applicable | target/certification/project |
| AC-03 | CERT-TIMING-SOAK | TIME-HIL-01 | pending | target/certification/evidence.json |
| AC-04 | CERT-MAX-ADMITTED-GRAPH<br>mixing_graph_rejects_cycle |  | not_applicable | target/certification/evidence.json |
| AC-05 | take_changes_program_pixels_on_same_boundary | AUDIO-HIL | pending | target/certification/runtime |
| AC-06 | CERT-FAULT-SINK<br>CERT-FAULT-DISK-FULL<br>CERT-FAULT-NIC-OUTAGE | DIST-HIL | pending | target/certification/evidence.json |
| AC-07 | CERT-COMMAND-FLOOD-REPLAY |  | not_applicable | target/certification/evidence.json |
| AC-08 | CERT-MAX-ADMITTED-GRAPH<br>CERT-GPU-DEVICE-LOSS | GPU-HIL | pending | target/certification/evidence.json |
| AC-09 | CERT-TIMING-SOAK | 24H-WALL-SOAK | pending | target/certification/evidence.json<br>target/certification/manual/24h |
| AC-10 | CERT-TIMING-SOAK | TIME-HIL-06<br>AUDIO-HIL | pending | target/certification/evidence.json<br>target/certification/hil/timing |
| AC-11 | adapter contract tests | NDI-HIL<br>OMT-HIL<br>DECKLINK-HIL | pending | target/certification/hil/interop |
