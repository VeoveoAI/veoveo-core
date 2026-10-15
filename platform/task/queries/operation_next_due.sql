-- veoveo: all tenants (the due pass reads the server's next due time across every tenant)
-- The earliest time server $1's due pass has work: a parked Task's next check or a
-- pending operation's deadline.
SELECT least(
    (
        SELECT min(o.next_check_at)
        FROM tasks.task_operation AS o
        JOIN tasks.task AS parked
            ON parked.id = o.task_id AND parked.parked_on = o.operation_key
        WHERE o.server = $1 AND o.next_check_at IS NOT NULL
            AND o.state IN ('dispatching', 'submitted') AND parked.status = 'waiting'
    ),
    (
        SELECT min(deadline)
        FROM tasks.task_operation
        WHERE server = $1 AND state IN ('dispatching', 'submitted')
    )
) AS "due"
