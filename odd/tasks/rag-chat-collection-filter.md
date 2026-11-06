# Research chat: choose which collections to search

**Status:** done in PR #227. Requested by the owner 2026-11-05.

- [x] Let the research Chat search **one or several chosen collections**,
      the same way the research Agent already does.

## Done

Implemented in PR #227 as a collection menu next to **Corpus / Biblioteca /
Ambos** instead of reusing the Agent's checklist (owner's decision,
2026-11-05). The filter is applied in each retrieval leg before its cap, and
chapter 13.1 of the user manual is updated. The menu's **all** row toggles:
it checks every collection, and unchecks them all when every one is checked;
with none checked the Chat asks for at least one instead of sending.

## Before this change

The Chat searched all collections together. The manual said so (chapter 13.1:
«El Chat no permite elegir una colección») and the code confirmed it:
`apps/desktop/src/views/RagChatView.svelte` showed which collection each
source comes from, but did not filter by collection.

Workarounds by then: use the research **Agent**, which has the selector, or
name the collection in the question (helps, not guaranteed).

## Notes

- The Agent's collection selector already existed. The Chat got its own menu
  in the scope row (see above) rather than the Agent's checklist, by the
  owner's decision on 2026-11-05.
- Chapter 13 of the user manual is updated (13.1).
