CREATE TABLE llm_results_v2 (
  id TEXT PRIMARY KEY,
  target_id TEXT NOT NULL,
  target_type TEXT NOT NULL CHECK(target_type IN ('asset', 'item', 'collection', 'unknown')),
  job_type TEXT NOT NULL,
  result TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

INSERT INTO llm_results_v2 (id, target_id, target_type, job_type, result, created_at)
SELECT
  lr.id,
  lr.target_id,
  CASE
    WHEN EXISTS (SELECT 1 FROM assets a WHERE a.id = lr.target_id) THEN 'asset'
    WHEN EXISTS (SELECT 1 FROM items i WHERE i.id = lr.target_id) THEN 'item'
    WHEN EXISTS (SELECT 1 FROM collections c WHERE c.id = lr.target_id) THEN 'collection'
    ELSE 'unknown'
  END,
  lr.job_type,
  lr.result,
  CASE
    WHEN lr.created_at > 0 AND lr.created_at < 1000000000000 THEN lr.created_at * 1000
    ELSE lr.created_at
  END
FROM llm_results lr;

DROP TABLE llm_results;
ALTER TABLE llm_results_v2 RENAME TO llm_results;

CREATE INDEX IF NOT EXISTS idx_llm_results_target ON llm_results(target_id);
CREATE INDEX IF NOT EXISTS idx_llm_results_target_typed
ON llm_results(target_type, target_id, job_type)
