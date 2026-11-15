# Release 1.8.0 (local preparation) + EntropIA Pro Store MSIX

Prepare the 1.8.0 release locally on branch `release/1.8.0` (no push, no tag) and add a
Microsoft Store MSIX for EntropIA Pro, built from scratch (generated AppxManifest + makeappx).

## Specs

- S1: "dejar todo preparado para la release de la version 1.8.0, por ahora solo en local" — every version declaration says 1.8.0 (`pnpm version:check` passes); nothing is pushed, tagged or published.
- S2: "agregando el msix para EntropIA Pro" — a local script produces an unsigned Store MSIX for Pro (channel: Microsoft Store, which signs it), from the Pro MSI layout, with a manifest generated from scratch (no Hyper-V capture).
- S3: Pro's Partner Center identity (Name, Publisher, PublisherDisplayName) is not known yet ("Tiene el espacio y el nombre, no cargamos nada aun, esta 1.8.0.0 sería la primera"): the script takes it as mandatory parameters, never a placeholder default. Store version 1.8.0.0.
- S4: the Pro package carries the assistant connector reachable through an execution alias that does not collide with Lite's (`entropia-agent-connector.exe`); the app names that same alias when packaged.
- S5: scope "Bump + MSIX + commits en rama": commits on `release/1.8.0`, no local Pro build.

## Tasks

- T1 [S1] bump to 1.8.0 via `pnpm version:bump 1.8.0` — inline — commit: 9f30d122 (version:check OK)
- T2 [S4] Pro connector alias `entropia-pro-agent-connector.exe` in `automation/connector.rs` — inline — commit: ad2c9897 (Lite cargo tests 9/9; Pro variant not compiled: MNN)
- T3 [S2,S3,S4] Pro Store MSIX script + manifest helper + Pester tests + version.mjs registration + docs — inline + worker (docs) — commit: 2559c59a (Pester 50/50 incl. real makeappx pack; version-consistency 11/11)
- T4 [S1] 1.8.0 release notes draft + release checklist — inline — commit: 52577332 (worker)

## Log

- L1: "quiero que dejes todo preparado para la release de la version 1.8.0, por ahora solo en local y agregando el msix para EntropIA Pro"
- L2: answers — identity: "Tiene el espacio y el nombre, no cargamos nada aun, esta 1.8.0.0 sería la primera"; channel: Microsoft Store; method: manifest from scratch; scope: Bump + MSIX + commits en rama.
- L3: pre-existing uncommitted edits in `.gitignore` and `Cargo.lock` (entropia-agent `source` line) are not part of this feature and stay out of its commits.
- L4: verification — `ci-rust-quality-workflow.Tests.ps1` fails one case (Cargo.lock must pin entropia-agent to a GitHub commit) only because of the pre-existing uncommitted Cargo.lock edit from L3; committed Cargo.lock keeps the pin.
