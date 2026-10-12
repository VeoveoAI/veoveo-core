BEGIN TRANSACTION;
            CREATE audit_domain_fixture:two SET revision = 2;
            fn::audit::append($audit_rows); COMMIT TRANSACTION;
