DEFINE FUNCTION fn::kernel::identity::oauth_client_exists($client: string) -> bool {
    RETURN array::len(SELECT id FROM oauth_client WHERE client_id = $client LIMIT 1) > 0;
} PERMISSIONS FULL;
