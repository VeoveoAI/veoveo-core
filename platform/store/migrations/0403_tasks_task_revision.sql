-- Every update advances the revision and update time. Identity, ownership and the
-- request stay as the Task was created.
CREATE FUNCTION tasks.task_revision() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.id, NEW.server, NEW.tenant, NEW.owner_tenant, NEW.owner_principal,
        NEW.owner_kind, NEW.owner_issuer, NEW.owner_subject, NEW.profile, NEW.data_labels,
        NEW.work_context, NEW.authority, NEW.task_type, NEW.recovery_class, NEW.input,
        NEW.idempotency_key, NEW.ttl_ms, NEW.poll_interval_ms, NEW.created_at,
        NEW.retention_expires_at)
        IS DISTINCT FROM
       (OLD.id, OLD.server, OLD.tenant, OLD.owner_tenant, OLD.owner_principal,
        OLD.owner_kind, OLD.owner_issuer, OLD.owner_subject, OLD.profile, OLD.data_labels,
        OLD.work_context, OLD.authority, OLD.task_type, OLD.recovery_class, OLD.input,
        OLD.idempotency_key, OLD.ttl_ms, OLD.poll_interval_ms, OLD.created_at,
        OLD.retention_expires_at)
    THEN
        RAISE EXCEPTION 'task % identity and request are immutable', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.revision := OLD.revision + 1;
    NEW.updated_at := now();
    RETURN NEW;
END;
$$;

CREATE TRIGGER task_revision BEFORE UPDATE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION tasks.task_revision();
