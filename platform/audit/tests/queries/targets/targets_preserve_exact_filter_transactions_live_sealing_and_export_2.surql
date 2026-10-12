BEGIN TRANSACTION; CREATE audit_target_fixture:rollback SET revision=1; fn::audit::append($audit_rows); COMMIT TRANSACTION;
