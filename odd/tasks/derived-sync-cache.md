# Derived sync cache (do not pay twice across devices)

Client side of `odd/plans/plan-sync-bibliografia-derivada.md` (revision 7): a content-addressed cache of paid
results (embeddings, OCR pages) that travels through EntropIA Cloud. Branch `feat/derived-sync-cache`, board
card T-56. The plan is the design reference; read the sections each task names.

## Specs

- **S1** (user, L1): "es pagar dos veces o mas, si lo abro en un tercer dispositivo seria pagar tre veces pro lo
  mismo, todo tendria que viajar via sync una ves que en un dispositivo ya se proceso el amterial sea
  colecicones o bibliotecas".
- **S2** (decisions, L3): the plan's recommendations are the decisions: D1 cache tables live in `sync_*`
  created by `ensure_sync_schema`, no migration; D2 the OCR key is the sha256 of the file; D4 vectors travel
  as exact f32; D6 a row of another account is never used nor pushed (plan §2.9); D8 dedicated channel
  `bibliography-derived-v1` (plan §2.5 option b).
- **S3** (plan §8): "Con dos dispositivos en la misma cuenta, el mismo Zotero y la misma configuración de
  embeddings y OCR: después de que A termina, B completa "fichas N/N · pasajes M/M" **sin ninguna llamada paga
  a OpenRouter ni a GLM-OCR** para el material que A ya procesó. Un tercer dispositivo, C, se comporta igual que
  B."
- **S4** (plan §8): "**Material ya procesado antes de actualizar:** si A ya había procesado la biblioteca antes
  de tener este canal, tras actualizar y sembrar (§2.3b) B también la completa sin gasto, salvo las páginas de
  OCR con respuesta vacía que dejaron la fila nativa en archivos anteriores al canal, que no tienen evidencia y
  se pagan una vez."
- **S5** (plan §8): "**Primer arranque de B:** con sesión y capability aún desconocida, B espera (hasta el
  tope) antes de la primera llamada paga, en vez de pagar de inmediato." and "**Frescura:** si A sube
  resultados después de que B terminó el catch-up y B recibe el catálogo de Zotero antes del siguiente pull, B
  los baja antes de pagar y no hace llamadas por ellos."
- **S6** (plan §8): "**Colecciones:** B completa los fragmentos de las Colecciones de A sin llamadas a
  `embed_text`."
- **S7** (plan §8): "**Cuentas:** tras un cambio de cuenta no se empuja ni se usa ninguna fila de la cuenta
  anterior, el logout no borra las filas y el backfill de la cuenta nueva no recrea bajo ella el OCR de la
  anterior. Una respuesta que salió con la sesión de la cuenta anterior no instala filas ni mueve cursor,
  capability o `pushed` de la nueva."
- **S8** (plan §8): "**Sin sesión:** un dispositivo sin sesión que reprocesa material ya cacheado por el dueño
  del archivo no hace llamadas pagas por él."

## Tasks

Every task: test-first, no real archive (tests use in-memory/temp DBs; manual runs only with
`ENTROPIA_DEV_PROFILE`), one work-unit commit. Plan section and its WU tests are the task's checklist.

| ID | Task | Specs | Plan | Route | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Cache tables in `ensure_sync_schema` + account semantics (marks, `''` adoption, logout reset, effective account) | S2, S7, S8 | §2.2, §2.9, WU1 (account tests) | worker | c8f951bf |
| T2 | Versioned OCR contract | S2 | D3, WU3 | worker | dde1d82a |
| T3 | Write paid results to the cache in the same transaction (item, chunk, OCR outcomes incl. empty) | S3 | §2.3, WU1 | worker | ee754978 |
| T4 | Read before paying: bibliography vectors (item + chunk), effective account, width validation | S3, S8 | §2.4, WU2 | worker | 75223749 |
| T5 | Read before paying: OCR pages, cached pages cut out of PDF windows | S3 | §2.4, WU2 | worker | — |
| T6 | Collections: read/write the cache in `processing/embedding.rs` | S6 | §2.8, WU2b | worker | — |
| T7 | Backfill of already-paid results (owner account only) | S4, S7 | §2.3b, WU1b | worker | — |
| T8 | Transport `bibliography-derived-v1` (client, vs `SyncApi` mock), frozen request scope | S2, S3, S7 | §2.5, §2.9, WU4 | worker | — |
| T9 | Pull-before-pay gate, freshness barrier, footer text | S5 | §2.6, WU5 | worker | — |
| T10 | Server support (capability, wire tables, idempotency, quota) | S3 | §2.7, WU6 | blocked: server code is not in this repo | — |
| T11 | Two-device verification on dev profiles against staging | S3-S8 | WU7 | blocked: T10 | — |

## Log

- **L1** (user, 2026-10-27): "por qué si en mi otra pc antropía lite ya tiene toda la biblio actualizada acá esta
  en proceso, no debera estar todo sincronizado via sincronizacion de entropia cloud??? o la bibliteca no se
  sincroniza??? incronización al día · Bibliografía: fichas 1934/2810 · pasajes 0/1050" then "es un acarencia,
  es pagar dos veces o mas, si lo abro en un tercer dispositivo seria pagar tre veces pro lo mismo, todo tendria
  que viajar via sync una ves que en un dispositivo ya se proceso el amterial sea colecicones o bibliotecas".
- **L2** (user): "armá el plan que se lo voy a pasar a unos expertos para que lo evaluen". Plan written,
  two Judgment Day runs (first ESCALATED on JD-B-001, second APPROVED at rev 6), rev 7 committed as
  0473fd8c and pushed on `docs/plan-sync-bibliografia-derivada`; RDD review approved and acknowledged.
- **L3** (user, 2026-10-27): asked whether to wait for the experts or take the plan's recommendations as the
  decisions, the user answered "arrancá". D3, D5 and D7 stay open; T2 and T9 fix working values and record
  them here.
- **L4** (T2, D3 working value): OCR contract `ocr-v<version>:<provider>:<model>:p<version>` from
  `bibliography/ocr_contract.rs` (`OCR_PAGE_CONTRACT_VERSION` = 1, prompt/parser fingerprint pinned by a test).
  Pro (`local-ml`/`paddle-ocr`) compile not verified on this notebook: its `cmake` is a broken Python shim.
- **L5** (user, 2026-10-27, mid T1): "metele con todo, me voy a dormir, espeor que tengas todo terminado apra
  cuando me levante".
- **L6** (T3 verification): plan assumption S-f is false. `ocr_markup_to_text` is not idempotent
  ("<p>a &lt;b&gt; c</p>" → "a <b> c" → "a c"), so the cache stores the RAW provider answer and the reader
  replays `settled_page_row` on it. The first T3 draft double-converted and changed published text; fixed before
  commit with a RED/GREEN test. The reprocess reuse path the plan cites (`processing.rs:3382-3392`) relies on the
  same false assumption; pre-existing, outside this feature, not changed.
