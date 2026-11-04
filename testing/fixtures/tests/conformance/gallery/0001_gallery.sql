-- A conforming server schema whose rows carry pgvector vectors from the installation's
-- `_extensions` schema.
CREATE TABLE gallery.picture (
    tenant uuid NOT NULL,
    id uuid NOT NULL,
    embedding _extensions.vector(3),
    PRIMARY KEY (tenant, id)
);

CREATE INDEX picture_embedding ON gallery.picture
    USING hnsw (embedding _extensions.vector_cosine_ops);

COMMENT ON TABLE gallery.picture IS 'Retention: until the tenant removes the picture.';
