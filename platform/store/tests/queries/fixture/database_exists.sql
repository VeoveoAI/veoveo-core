SELECT EXISTS (SELECT FROM pg_database WHERE datname = $1) AS "exists!"
