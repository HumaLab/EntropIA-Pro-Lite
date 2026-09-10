export interface TopicEditorProps {
  /** Current topic names for this item (normalized UPPERCASE). */
  topics?: string[]
  /** Autocomplete suggestions from the global topic pool. */
  suggestions?: string[]
  /** Called when topics change (added or removed). Returns the full updated list. */
  onchange?: (topics: string[]) => void | Promise<void>
}
