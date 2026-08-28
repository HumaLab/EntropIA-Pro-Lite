import type { IconButtonProps } from '../IconButton'

export interface SearchClearButtonProps
  extends Omit<IconButtonProps, 'active' | 'children' | 'label' | 'size' | 'type' | 'variant'> {
  label: string
  title?: string
}
