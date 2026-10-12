CREATE FUNCTION public.fixture_seal_failure() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'fixture seal failure';
END
$$
