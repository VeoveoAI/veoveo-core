-- A registration records when the Gateway first found its login without a live kernel
-- instance. orphaned_at is NULL while the login holds one. The Gateway sets it once when
-- it finds none, clears it when an instance appears, and removes the registration with
-- its login once orphaned_at is older than the installation's grace period, so an
-- installation whose servers stay alive writes nothing while it is idle.
ALTER TABLE gateway.registration
    DROP COLUMN live_at,
    ADD COLUMN orphaned_at timestamptz;

-- registration_change fires on inserts, deletes and changes of the metadata or software
-- statement only, so setting or clearing orphaned_at wakes no listener.
