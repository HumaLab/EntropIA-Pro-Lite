-- Settle a unit's blocked dependents the moment it ends, instead of every
-- claim rescanning every blocked unit. The scan cost O(blocked) inside the
-- write lock on each claim: 0.8 s per page with 200k pages queued, which
-- starved every other writer for the whole OCR phase.
--
-- Same rules as settle_blocked_dependents (processing/repository.rs), which
-- stays as the one-off repair run at recovery: a succeeded dependency moves
-- its blocked dependents to pending; a failed or cancelled one fails them.
-- The trigger fires for every transition path and every process, so no Rust
-- call site can forget it.

-- claim_next walks pending units in id order and stops at the first runnable
-- one; without (state, id) it gathered and sorted every pending unit per claim.
CREATE INDEX IF NOT EXISTS idx_processing_tasks_state_id
  ON processing_tasks(state, id);

CREATE INDEX IF NOT EXISTS idx_processing_batch_tasks_dependency
  ON processing_batch_tasks(dependency_task_id)
  WHERE dependency_task_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS processing_tasks_settle_dependents
AFTER UPDATE OF state ON processing_tasks
WHEN NEW.state IN ('succeeded', 'failed', 'cancelled') AND OLD.state IS NOT NEW.state
BEGIN
  UPDATE processing_attempts SET outcome = 'failed', finished_at = strftime('%s', 'now') * 1000
   WHERE NEW.state <> 'succeeded' AND outcome = 'open'
     AND task_id IN (
       SELECT l.task_id FROM processing_batch_tasks l
         JOIN processing_tasks t ON t.id = l.task_id
        WHERE l.dependency_task_id = NEW.id AND t.state = 'blocked');
  UPDATE processing_tasks
     SET state = 'failed', outcome = 'dependency_failed', last_error_code = 'dependency_failed',
         last_error_message = 'dependency ' || NEW.id || ' ended as ' || NEW.state,
         updated_at = strftime('%s', 'now') * 1000
   WHERE NEW.state <> 'succeeded' AND state = 'blocked'
     AND id IN (SELECT task_id FROM processing_batch_tasks WHERE dependency_task_id = NEW.id);
  UPDATE processing_tasks
     SET state = 'pending', stage = '', updated_at = strftime('%s', 'now') * 1000
   WHERE NEW.state = 'succeeded' AND state = 'blocked'
     AND id IN (SELECT task_id FROM processing_batch_tasks WHERE dependency_task_id = NEW.id);
END;
