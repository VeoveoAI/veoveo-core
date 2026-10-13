-- Every table states how long its rows stay and what removes them.
COMMENT ON TABLE identity.enterprise IS
    'Retention: for the life of the installation.';

COMMENT ON TABLE identity.tenant IS
    'Retention: for the life of the installation.';

COMMENT ON TABLE identity.principal IS
    'Retention: for the life of the installation.';

COMMENT ON TABLE identity.work_context IS
    'Retention: while the active control-plane revision defines it; a publication '
    'deletes every Work Context the new revision omits.';

COMMENT ON TABLE identity.work_context_grant IS
    'Retention: until a publication replaces its Work Context''s grants, or deletes '
    'the Work Context.';

COMMENT ON TABLE identity.work_context_membership IS
    'Retention: until a publication replaces its Work Context''s membership rules, or '
    'deletes the Work Context.';
