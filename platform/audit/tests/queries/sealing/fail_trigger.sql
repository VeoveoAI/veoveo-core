CREATE TRIGGER fixture_seal_failure BEFORE INSERT ON audit.block
    FOR EACH ROW EXECUTE FUNCTION public.fixture_seal_failure()
