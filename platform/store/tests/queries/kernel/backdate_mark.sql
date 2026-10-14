-- Move an instance's free-lock mark back by $2, as if a reaper had marked it that long ago.
UPDATE kernel.instance SET lock_free_since = now() - $2::interval WHERE id = $1
