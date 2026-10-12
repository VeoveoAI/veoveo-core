DEFINE FUNCTION fn::kernel::identity::enabled_user($tenant: record<tenant>, $principal: record<principal>) -> bool {
    RETURN count(SELECT id FROM principal
        WHERE id = $principal AND tenant = $tenant AND enabled = true AND kind = 'user' LIMIT 1) > 0;
} PERMISSIONS FULL;
