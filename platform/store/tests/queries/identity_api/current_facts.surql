RETURN fn::kernel::identity::tenant_current($enterprise, $tenant);
RETURN fn::kernel::identity::principal_current($tenant, $principal);
RETURN fn::kernel::identity::tenant_matches($enterprise, $tenant, $slug);
