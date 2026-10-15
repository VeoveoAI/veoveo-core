-- An owner schema that breaks one schema rule: a table without a retention comment.
CREATE TABLE stockroom.bin (
    id uuid PRIMARY KEY,
    label text NOT NULL
);
