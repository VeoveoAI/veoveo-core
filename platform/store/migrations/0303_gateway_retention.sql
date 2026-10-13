-- Every table states how long its rows stay and what removes them.
COMMENT ON TABLE gateway.control_revision IS
    'Retention: for the life of the installation; a revision never changes.';

COMMENT ON TABLE gateway.control_object IS
    'Retention: as long as its revision.';

COMMENT ON TABLE gateway.control_active IS
    'Retention: one row for the life of the installation.';

COMMENT ON TABLE gateway.jwt_revocation IS
    'Retention: until the revoked JWT expires; the Gateway prunes expired revocations '
    'in batches.';

COMMENT ON TABLE gateway.replay_id IS
    'Retention: until the identifier expires; the Gateway prunes expired identifiers in '
    'batches, and a new claim may take an expired row.';

COMMENT ON TABLE gateway.authorization_request IS
    'Retention: until the identity provider''s callback consumes it or it expires; the '
    'Gateway prunes expired requests in batches.';

COMMENT ON TABLE gateway.authorization_code IS
    'Retention: until it expires, redeemed or not; the Gateway prunes expired codes in '
    'batches.';

COMMENT ON TABLE gateway.refresh_family IS
    'Retention: until it expires; the Gateway prunes expired families in batches, and '
    'their tokens go with them.';

COMMENT ON TABLE gateway.refresh_token IS
    'Retention: until it or its family expires; the Gateway prunes expired tokens in '
    'batches and clears a delivery envelope when its window ends.';

COMMENT ON TABLE gateway.namespace_claim IS
    'Retention: until an administrator releases the namespace.';

COMMENT ON TABLE gateway.registration IS
    'Retention: until the server deregisters, or until its login has held no live '
    'kernel instance for the grace period; the Gateway then removes it with its login.';

COMMENT ON TABLE gateway.task_route IS
    'Retention: until it expires or a publication deletes its Work Context; the Gateway '
    'prunes expired routes in batches.';
