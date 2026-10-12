BEGIN TRANSACTION;
LET $current = (SELECT * FROM ONLY $window);
IF $current != NONE AND $current.end <= time::now() AND $current.reads = $reads {
    fn::audit::append($audit_rows);
    DELETE ONLY $window;
};
COMMIT TRANSACTION;
