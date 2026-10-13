-- veoveo: all tenants (a server login probing the audit table it may not read)
SELECT count(*) FROM audit.record
