-- One Work Context with its grants and membership rules, read in one snapshot.
SELECT
    w.id AS "id: WorkContextId",
    w.tenant AS "tenant: TenantId",
    w.context_key,
    w.title,
    w.policy_revision,
    w.owner_kind AS "owner_kind: GrantSubjectKind",
    w.owner_key,
    w.classification,
    w.data_labels,
    (
        SELECT coalesce(
            jsonb_agg(
                jsonb_build_object(
                    'subject_kind', g.subject_kind,
                    'subject_key', g.subject_key,
                    'permission', g.permission
                )
                ORDER BY g.ordinal
            ),
            '[]'::jsonb
        )
        FROM identity.work_context_grant AS g
        WHERE g.work_context = w.id
    ) AS "initial_grants!: Json<Vec<WorkContextInitialGrantRecord>>",
    (
        SELECT coalesce(
            jsonb_agg(
                jsonb_build_object(
                    'level', m.level,
                    'principals', m.principals,
                    'groups', m.groups,
                    'roles', m.roles,
                    'oauth_clients', m.oauth_clients
                )
                ORDER BY m.ordinal
            ),
            '[]'::jsonb
        )
        FROM identity.work_context_membership AS m
        WHERE m.work_context = w.id
    ) AS "memberships!: Json<Vec<WorkContextMembershipRuleRecord>>",
    w.created_at,
    w.updated_at
FROM identity.work_context AS w
WHERE w.tenant = $1 AND w.context_key = $2
