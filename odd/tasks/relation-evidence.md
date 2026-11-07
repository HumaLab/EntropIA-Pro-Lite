# Relation evidence: reconstructed triples in Relaciones 3D passages

Follow-up to `odd/tasks/relations-3d-navigation-params.md` (PR #254). Branch
`feat/relation-evidence-scoring`.

## Specs

- **S1 Reconstructed triples are located and explained.** "no siempre Sujeto - Relación - Objeto son literales proque el LLM rearma los tripletes interpretando pro contexto lo que rendunda en que muchos tripretes no se muestran subrayados cuando hacemos clic, en ocaciones solo hay Sujeto en otras solo Objeto e imagino que en otras solo relación, habrái que ver la forma de tener identificacion subrayada de entidades nombradas literales y entridades recosntruidas o en el casod e que un subrayado difuso no es posible se prodia agraggar en la entidad y o relacion subrayada la nota explicativa de que los triupetes fueron cosntruidos en abs a al informacion de tal ocual parrafo u oración"
  - Examples (user): "la Federación de Obreros en Construcciones Navales declaró un paro de 48 horas en la industria naval de Capital y Gran Buenos Aires en solidaridad con los obreros de F.A.N.U." / "la Federación de Obreros en Construcciones Navales mantuvo cierto control sobre el trabajo de los talleres navales" (text reads "la Federación mantuvo su actividad, … y mantuvo cierto control sobre el trabajo de los talleres navales").
  - Decision (L2): score sentences by evidence of all three parts (whole phrase literal, entity head/content tokens, predicate content words; diacritic- and case-insensitive); an occurrence is the best supporting sentence (or two adjacent sentences); sentences with no evidence are dropped; steps are supporting sentences, never loose single-entity hits elsewhere. Literal matches: solid underline; partial/reconstructed: dashed underline; a note says which parts are literal and which were inferred, and that the triple was built from that sentence.
- **S2 Store evidence for future extractions.** Decision (L2): the triple prompt also returns the source sentence; it is stored (nullable, migration) and used for exact highlighting; existing triples keep using S1.

- **S3 Selected node stands out.** "cuando se hace clic sobr eun nodo d ela red elnodo quede más brillante  e intenso con un reborde de on color contrastente y que sus vinculos se resalten tmabien"

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1 | inline — sentence scoring locator in `relation-occurrences.ts`, reconstructed role + note in highlight/ItemView/OccurrenceStepper, tests | done | (T1 commit) |
| T3 | S3 | inline — selected node brighter with a contrasting outline, its links highlighted (`RelationGraph3D.svelte`) | done | (T3 commit) |
| T2 | S2 | needs explorer (sync contract of `triples`, `schema_tag`, migration path) then implementation + independent verify (Risk items 1, 3) | blocked: subagents unavailable (provider quota) | |

## Log

- **L1** (user, verbatim): "encontre un problema, entiendo que no es un error pero habría que resolverlo de la mejor manera posible, pasa esto, no siempre Sujeto - Relación - Objeto son literales proque el LLM rearma los tripletes interpretando pro contexto lo que rendunda en que muchos tripretes no se muestran subrayados cuando hacemos clic, en ocaciones solo hay Sujeto en otras solo Objeto e imagino que en otras solo relación, habrái que ver la forma de tener identificacion subrayada de entidades nombradas literales y entridades recosntruidas o en el casod e que un subrayado difuso no es posible se prodia agraggar en la entidad y o relacion subrayada la nota explicativa de que los triupetes fueron cosntruidos en abs a al informacion de tal ocual parrafo u oración ¿como lo ves? te pñaso un apr de ejemplso de tripletes cuyos links solo muestran una entidad: la Federación de Obreros en Construcciones Navales declaró un paro de 48 horas en la industria naval de Capital y Gran Buenos Aires en solidaridad con los obreros de F.A.N.U. la Federación de Obreros en Construcciones Navales mantuvo cierto control sobre el trabajo de los talleres navales" Screenshots: solo-objeto.png (only "el trabajo de los talleres navales" underlined, "Aparición 1 de 3", note "Referencia aproximada: solo se encontró una entidad, no el vínculo."), solo-sujeto.png (step lands on an unrelated paragraph where only the subject appears).
- **L2** (user decision): "Eso + guardar evidencia en extracciones futuras" — S1 scoring + two underline styles + note, plus S2 evidence storage.
- **L3** Evidence so far: triples written only by `replace_triples_for_asset` (`apps/desktop/src-tauri/src/llm/mod.rs:1354`, DELETE+INSERT per item/asset, columns id,item_id,asset_id,subject,predicate,object,created_at); parsed into `LlmTriple` (`llm/mod.rs:1038`, subject/predicate/object with Spanish aliases); `triples` is captured by sync (`sync/capture.rs:26,55`); `schema_tag` read in `sync/derived_transport.rs`. Latest migration 0061.
- **L4** (user, verbatim): "otra cosa menor, quiero que cuando se hace clic sobr eun nodo d ela red elnodo quede más brillante  e intenso con un reborde de on color contrastente y que sus vinculos se resalten tmabien"
