# Research chat: choose which collections to search

**Status:** pending, not started. Requested by the owner 2026-11-05.

- [ ] Let the research Chat search **one or several chosen collections**, the
      same way the research Agent already does.

## Today

The Chat always searches all collections together. The manual says so
(chapter 13.1: «El Chat no permite elegir una colección») and the code
confirms it: `apps/desktop/src/views/RagChatView.svelte` shows which
collection each source comes from, but does not filter by collection.

Workarounds meanwhile: use the research **Agent**, which has the selector, or
name the collection in the question (helps, not guaranteed).

## Notes

- The Agent's collection selector already exists: reuse it, don't build a
  second one.
- Update chapter 13 of the user manual when it ships.
