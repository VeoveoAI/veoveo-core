-- A logical server may have independent admitted deployments sharing its schema.
-- Queue affinity is not a data-cloning or database-role boundary. Existing development
-- Tasks must be reset before applying this required-identity hard cut.
ALTER TABLE tasks.task ADD COLUMN deployment uuid NOT NULL
    CONSTRAINT task_deployment_v7 CHECK (
        uuid_extract_version(deployment) = 7 AND substr(deployment::text, 20, 1) IN ('8', '9', 'a', 'b')
    );

CREATE FUNCTION tasks.task_deployment_immutable() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.deployment IS DISTINCT FROM OLD.deployment THEN
        RAISE EXCEPTION 'task % keeps its deployment', OLD.id USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION tasks.task_deployment_immutable() FROM PUBLIC;
CREATE TRIGGER task_deployment_immutable BEFORE UPDATE OF deployment ON tasks.task
FOR EACH ROW EXECUTE FUNCTION tasks.task_deployment_immutable();

DROP INDEX tasks.task_idempotency;
CREATE UNIQUE INDEX task_idempotency ON tasks.task (
    server, deployment, tenant, owner_principal, surface, idempotency_key
) WHERE idempotency_key IS NOT NULL;

DROP INDEX tasks.task_owner_page;
CREATE INDEX task_owner_page ON tasks.task (
    server, deployment, tenant, owner_principal, surface, created_at, id
);

DROP INDEX tasks.task_claimable;
CREATE INDEX task_claimable ON tasks.task (server, deployment, created_at, id)
WHERE status = 'queued' OR (status = 'cancel_requested' AND owner_instance IS NULL);
