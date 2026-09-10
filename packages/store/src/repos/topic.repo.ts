import { eq, and, like, sql } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { topics, itemTopics } from '../schema'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type Topic = typeof topics.$inferSelect
export type ItemTopic = typeof itemTopics.$inferSelect

export class TopicRepo {
  constructor(private db: DrizzleClient) {}

  // -------------------------------------------------------------------------
  // Topic CRUD
  // -------------------------------------------------------------------------

  /** Find or create a topic by name. Normalizes to UPPERCASE. */
  async findOrCreate(name: string): Promise<Topic> {
    const normalized = name.trim().toUpperCase()
    // Try to find existing
    const existing = await this.db.select().from(topics).where(eq(topics.name, normalized))

    if (existing.length > 0) {
      return existing[0]!
    }

    // Create new
    const now = Date.now()
    const newTopic: Topic = {
      id: crypto.randomUUID(),
      name: normalized,
      createdAt: now,
    }
    await this.db.insert(topics).values(newTopic)
    return newTopic
  }

  /** Find a topic by exact name (case-insensitive, normalizes to UPPERCASE). */
  async findByName(name: string): Promise<Topic | null> {
    const normalized = name.trim().toUpperCase()
    const rows = await this.db.select().from(topics).where(eq(topics.name, normalized))
    return rows[0] ?? null
  }

  /** Search topics by name prefix (for autocomplete). Returns up to `limit` results. */
  async search(prefix: string, limit = 20): Promise<Topic[]> {
    const normalized = prefix.trim().toUpperCase()
    if (!normalized) return []
    return this.db
      .select()
      .from(topics)
      .where(like(topics.name, `${normalized}%`))
      .limit(limit)
  }

  /** Get all topic names (for full autocomplete when input is empty). */
  async allNames(limit = 100): Promise<string[]> {
    const rows = await this.db
      .select({ name: topics.name })
      .from(topics)
      .orderBy(topics.name)
      .limit(limit)
    return rows.map((r) => r.name)
  }

  // -------------------------------------------------------------------------
  // Item ↔ Topic associations
  // -------------------------------------------------------------------------

  /** Get all topics for a given item. */
  async findByItemId(itemId: string): Promise<Topic[]> {
    const rows = await this.db
      .select({
        id: topics.id,
        name: topics.name,
        createdAt: topics.createdAt,
      })
      .from(itemTopics)
      .innerJoin(topics, eq(itemTopics.topicId, topics.id))
      .where(eq(itemTopics.itemId, itemId))
      .orderBy(topics.name)

    return rows
  }

  /** Add a topic to an item. Creates the topic if it doesn't exist. Normalizes name. */
  async addTopicToItem(itemId: string, topicName: string): Promise<Topic> {
    const topic = await this.findOrCreate(topicName)

    // Check if already associated
    const existing = await this.db
      .select()
      .from(itemTopics)
      .where(and(eq(itemTopics.itemId, itemId), eq(itemTopics.topicId, topic.id)))

    if (existing.length === 0) {
      await this.db.insert(itemTopics).values({
        id: crypto.randomUUID(),
        itemId,
        topicId: topic.id,
        createdAt: Date.now(),
      })
    }

    return topic
  }

  /** Add multiple topics to an item at once. Comma-separated or array of names. */
  async addTopicsToItem(itemId: string, names: string[]): Promise<Topic[]> {
    // Deduplicate and normalize
    const uniqueNames = [...new Set(names.map((n) => n.trim().toUpperCase()).filter(Boolean))]
    const result: Topic[] = []

    for (const name of uniqueNames) {
      const topic = await this.addTopicToItem(itemId, name)
      result.push(topic)
    }

    return result
  }

  /** Remove a topic from an item. Does NOT delete the topic itself. */
  async removeTopicFromItem(itemId: string, topicId: string): Promise<void> {
    await this.db
      .delete(itemTopics)
      .where(and(eq(itemTopics.itemId, itemId), eq(itemTopics.topicId, topicId)))
  }

  /** Remove all topics from an item. */
  async removeAllTopicsFromItem(itemId: string): Promise<void> {
    await this.db.delete(itemTopics).where(eq(itemTopics.itemId, itemId))
  }

  /** Count how many items use a given topic. */
  async countItemsForTopic(topicId: string): Promise<number> {
    const rows = await this.db
      .select({ count: sql<number>`count(*)` })
      .from(itemTopics)
      .where(eq(itemTopics.topicId, topicId))
    return rows[0]?.count ?? 0
  }
}
