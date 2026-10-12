-- A server-owned table for tests: notes that notify their schema's channel.
CREATE TABLE note (
    id uuid PRIMARY KEY,
    tenant uuid,
    body text NOT NULL
);

CREATE TRIGGER note_change
    AFTER INSERT OR UPDATE OR DELETE ON note
    FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('id', 'tenant');
