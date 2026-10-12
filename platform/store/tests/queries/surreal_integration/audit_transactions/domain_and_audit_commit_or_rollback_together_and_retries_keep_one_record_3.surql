BEGIN TRANSACTION;
            CREATE audit_domain_fixture:one SET revision = 1;
            fn::audit::append($audit_rows); COMMIT TRANSACTION;
