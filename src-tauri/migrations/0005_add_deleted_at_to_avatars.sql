-- Migration 0005: avatar tombstones participate in multi-device synchronization.
ALTER TABLE avatars ADD COLUMN deleted_at BIGINT;
