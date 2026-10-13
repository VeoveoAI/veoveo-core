-- A server schema that breaks each schema rule once.
CREATE TABLE parent (
    id uuid PRIMARY KEY
);

COMMENT ON TABLE parent IS 'Retention: as long as a child refers to it.';

-- No retention comment, and a foreign key without an index.
CREATE TABLE child (
    id uuid PRIMARY KEY,
    parent uuid NOT NULL REFERENCES parent (id)
);

-- No primary key, and a comment that states no retention.
CREATE TABLE loose (
    body text NOT NULL
);

COMMENT ON TABLE loose IS 'Free-form notes.';

-- A partial index on the foreign key whose predicate only excludes nulls serves the key.
CREATE TABLE optional_child (
    id uuid PRIMARY KEY,
    parent uuid REFERENCES parent (id)
);

CREATE INDEX optional_child_parent ON optional_child (parent) WHERE parent IS NOT NULL;

COMMENT ON TABLE optional_child IS 'Retention: removed with its parent.';
