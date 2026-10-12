SELECT * FROM provider_job WHERE task = $task AND provider = $provider AND tenant = $tenant
    AND fn::kernel::tasks::lifecycle(task, $server, tenant, $task_types, NONE) != NONE;
