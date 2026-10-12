BEGIN TRANSACTION;
            CREATE audit_domain_fixture:one SET revision = 1;
            fn::audit::append($audit_rows);
            THROW 'injected_domain_failure'; COMMIT TRANSACTION;
