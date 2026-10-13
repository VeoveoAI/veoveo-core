-- Every update advances the revision and the update time. A Task's identity, owner and
-- request stay as it was created.
CREATE FUNCTION tasks.task_revision() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.id, NEW.server, NEW.tenant, NEW.tenant_id, NEW.owner_tenant,
        NEW.owner_principal, NEW.principal_id, NEW.owner_kind, NEW.owner_issuer,
        NEW.owner_subject, NEW.surface, NEW.data_labels, NEW.work_context, NEW.authority,
        NEW.task_type, NEW.recovery_class, NEW.input, NEW.idempotency_key, NEW.ttl_ms,
        NEW.poll_interval_ms, NEW.created_at, NEW.retention_expires_at)
        IS DISTINCT FROM
       (OLD.id, OLD.server, OLD.tenant, OLD.tenant_id, OLD.owner_tenant,
        OLD.owner_principal, OLD.principal_id, OLD.owner_kind, OLD.owner_issuer,
        OLD.owner_subject, OLD.surface, OLD.data_labels, OLD.work_context, OLD.authority,
        OLD.task_type, OLD.recovery_class, OLD.input, OLD.idempotency_key, OLD.ttl_ms,
        OLD.poll_interval_ms, OLD.created_at, OLD.retention_expires_at)
    THEN
        RAISE EXCEPTION 'task % keeps its identity, owner and request', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.revision := OLD.revision + 1;
    NEW.updated_at := now();
    RETURN NEW;
END
$$;

CREATE TRIGGER task_revision BEFORE UPDATE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION tasks.task_revision();
