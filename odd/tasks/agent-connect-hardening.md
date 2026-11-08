# Assistant connection: hardening and a copy-paste instruction for any agent

Branch `feat/agent-connect-hardening`. Follows `agent-one-click-setup.md` (PR #257) and `mcp-named-pipe.md`.
Goal: pairing an assistant with EntropIA must not fail for users. The panel also offers one instruction to paste into any MCP-capable agent so that agent does the setup itself.

## Specs

- **S1. Claude absence wording.** «Conectar» for Claude must succeed when the Claude CLI reports `No MCP server named "entropia" in user scope` (or `local scope`) while removing a previous entry, even with other lines (certificate warnings) in stderr.
- **S2. Dev profile in every registration.** A connection made from a dev profile (`ENTROPIA_DEV_PROFILE=<name>`) writes `--client <id> --profile <name>` in every assistant's entry: one-click (Codex, Claude, OpenCode, Pi) and the manual lines the panel shows. Without a profile, nothing changes: `--client <id>` only.
- **S3. Diagnosis command.** `entropia-agent-connector --check --client <id> [--profile <name>]` performs a real round trip with the app (discovery, credential, `initialize`, `tools/list`). It prints one line: `ok: EntropIA <version> · <n> herramientas`, or the stable error code with its actionable message (`app_absent`, `credential_missing`, `unauthorized`, ...). It exits 0 on success and non-zero on failure, and never prints the token. `--help` documents it.
- **S4. Universal instruction.** User's words: «agreges en la conficguracion del asistnete una instruccion par acopia ry pegar en claude codex pi o opencode y que el usuario le peid aal agente que haga el trabajo» and «lo pienso como un compem,ento que tambien es util par aaegntes que no estam en etropia lsitados como antigravity qwen grok etc».
  - Decision: the client is created when the instruction is copied ("El cliente se crea al copiar").
  - The instruction carries the client id, the connector path, the profile when there is one, and the server entry in the common formats: `mcpServers` JSON, Codex TOML, and the CLI commands for Claude and Codex.
  - It asks the agent to add the server to its own MCP configuration, restart or reload if needed, verify with `--check`, and report the exact error instead of improvising.
  - It contains no secret: the token stays in the keyring.
- **S5. Unused clients are visible.** A client that was never used (`lastUsedAt` null) shows as «sin usar» in the panel and can be revoked like any other.
- **S6. Strong testing.** User's words: «testear fuerte para que sea irrompible y ningun usuario tnega probelmas»; testing scope: «todo lo qeu consideres necesario, no escatimes en recursos».
  - Automated failure cases for each assistant: CLI missing, config corrupt or read-only, duplicate or stale entry, paths with spaces and accents, dev profile, app closed, access off, revoked token.
  - Each failure yields a clear message and leaves no leftover client or partial config.
  - A real end-to-end run on this machine with Codex, Claude, OpenCode and Pi: connect, call a tool, disconnect, reconnect.

## Tasks

| ID | Specs | Route | Commit |
| --- | --- | --- | --- |
| T1 | S1, S2 (backend) | inline | 22140849 |
| T2 | S2 (frontend lines), S4, S5 | worker, finished inline | 4c6bc0d0 |
| T3 | S3 | worker | d41e3989 |
| T4 | S6 (automated failure cases) | inline (worker quota) | 3bc03a00 |
| T5 | S6 (real end-to-end) | parent | no code; evidence in L5 |

## Log

- L1 (user, 8 Nov 2026): «ahora quiero que lo vulvas a testear fuerte para que sea irrompible y ningun usuario tnega probelmas, o al menos reducir al maximo posibles errores y quiero que agreges en la conficguracion del asistnete una instruccion par acopia ry pegar en claude codex pi o opencode y que el usuario le peid aal agente que haga el trabajo, si lo ves viable armá l ainstruccion y disponibilizá las skills y o tools necesarias apr aque lso agente speudan hace rla conexion desde 0»
- L2 (user): pairing choice "El cliente se crea al copiar (Recomendado)"; testing: «todo lo qeu consideres necesario, no escatimes en recursos», with failure cases, `--check`, and the real four-assistant run selected.
- L3 (user): «lo pienso como un compem,ento que tambien es util par aaegntes que no estam en etropia lsitados como antigravity qwen grok etc».
- L4 (evidence): the dev test showed the Claude wording bug and the missing `--profile`. All four assistants connected green once `--profile relaciones3d` was added.
- L5 (evidence, 8 Nov 2026, dev profile relaciones3d):
  - `--check` returned `ok: EntropIA 1.4.0 · 22 herramientas` for the five clients.
  - `last_used_at` is recorded for the four used clients; «Grok», created by the user through the instruction, stays null, so the panel marks it «sin usar».
  - Real tool call `entropia_status`:
    - Claude Code (`claude -p`): ok, 28 colecciones, 2911 ítems.
    - OpenCode (`opencode run`, repo-built connector): ok.
    - Codex: not run, because the OpenAI account hit its usage limit.
    - Pi (`pi -p`): not run, because the print mode has no codemode executor. The interactive Pi showed the server connected.
  - From the harness shell, OpenCode with the installed MSIX 1.4.0 connector stayed `pending` while the repo-built connector connected. The user's own OpenCode sessions logged `mcp connected ... tools=22` with the installed one.
