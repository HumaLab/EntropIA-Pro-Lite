export interface CollectionCardProps {
  id: string
  name: string
  description?: string
  itemCount: number
  countLabel?: string
  updatedAt: number // unix ms timestamp
  locale?: string
  onclick?: () => void
  onedit?: () => void
  ondelete?: () => void
  editAriaLabel?: string
  deleteAriaLabel?: string
}
