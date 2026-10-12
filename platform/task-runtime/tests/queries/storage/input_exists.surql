RETURN { exists: fn::kernel::tasks::exists($task), matches: fn::kernel::tasks::input_matches($task, $expected_input) };
