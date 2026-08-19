-- Shared-value tables, tags, and interval-typed durations.
--
-- Three columns that held a bare value inline (`credit.role`, `track.rating`)
-- or a delimited list of them (`track.genre`) become links to tables of shared
-- values, so a value can be renamed in one place and referenced from many rows.
-- `track.rating` joins them, but points at a fixed set of standard ratings
-- rather than at whatever numbers the old column happened to hold.
-- The `duration` and `*_position` columns move from a bare count of seconds to
-- DuckDB's INTERVAL, so the unit lives in the type rather than in convention.
--
-- Links between tables are inferred by convention (see the `introspection`
-- crate): a UUID column named after another table references that table's `id`.
-- That is why each new link column is named exactly for its target table.
--
-- The statements are grouped into phases rather than by feature, because of a
-- DuckDB transaction rule this migration would otherwise trip over. Each
-- migration runs as a single transaction (see `db::run_migration`), and DuckDB
-- refuses to commit one that runs ALTER TABLE on a table it has already UPDATEd
-- ("Attempting to modify table X but another transaction has altered this
-- table"). Since `credit.role` and `track.rating` both need their old values
-- read, their columns swapped, and the new columns backfilled, every read comes
-- first, then every schema change, then every backfill.

-- === Phase 1: build the value tables and map the old values onto them =======
--
-- The scratch tables carry each row's key alongside the id of the shared value
-- it should end up pointing at, so the mapping survives dropping the column that
-- currently holds it. A row missing from a scratch table keeps a NULL link.

create table role (
  id uuid primary key,
  name text unique not null
);

insert into role (id, name)
select uuid(), role
from credit
where role is not null
group by role;

create table migration_0003_credit_role as
select c.track, c.artist, r.id as role
from credit c
join role r on r.name = c.role;

-- Ratings are the exception: rather than lifting whatever values happen to be
-- in the column, the table is seeded with a fixed set of four standard ratings,
-- and the old free-floating numbers are bucketed onto them. The old column held
-- a 0-to-5 score imported from file metadata, where 0 meant "unrated" and the
-- interesting distinctions all sat in the top point of the range. The ids are
-- hardcoded so that every library agrees on them.

create table rating (
  id uuid primary key,
  value float unique not null,
  symbol text unique,
  description text
);

insert into rating (id, value, symbol, description) values
('ed9a010a-124b-4aee-bce4-4889875142e8', 1, '🗑️', 'Skip'),
('ddd714b8-2d6a-4ff0-b280-74d8472116a7', 2, '✔️', 'Like'),
('7f592dd0-be55-4ef8-a946-1a11cd0d03b5', 3, '❤️', 'Love'),
('3e056915-37fc-4660-8db5-06c15572591a', 4, '🏆', 'Best');

create table migration_0003_track_rating as
select t.id as track, r.id as rating
from track t
join rating r on r.value = case
  when t.rating >= 5 then 4
  when t.rating >= 4.5 then 3
  when t.rating >= 4 then 2
  else 1
end
where t.rating is not null and t.rating <> 0;

-- Tags replace `track.genre`, which held one comma-joined string per track and
-- so could not represent a track's genres as separate values. Nothing is lifted
-- out of the old column: those strings were assembled by joining the file's
-- genre tags, and re-splitting them would invent boundaries for any genre that
-- legitimately contains a comma. The scanner repopulates `tag` from the files
-- themselves on the next scan.

create table tag (
  id uuid primary key,
  name text unique not null
);

create table track_tag (
  track uuid not null,
  tag uuid not null,
  primary key (track, tag)
);

-- === Phase 2: reshape the existing tables ==================================
--
-- `order` is a reserved word, so every reference to it must be quoted.

alter table credit rename column ord to "order";

alter table credit drop column role;
alter table credit add column role uuid;

alter table track drop column rating;
alter table track add column rating uuid;

alter table track drop column genre;

alter table file alter column duration set data type interval using to_seconds(duration);

alter table track alter column start_position set data type interval using to_seconds(start_position);

alter table track alter column end_position set data type interval using to_seconds(end_position);

-- === Phase 3: point the new link columns at the shared values ==============

update credit
set role = m.role
from migration_0003_credit_role m
where credit.track = m.track and credit.artist = m.artist;

update track
set rating = m.rating
from migration_0003_track_rating m
where track.id = m.track;

-- === Phase 4: discard the scratch tables ===================================

drop table migration_0003_credit_role;

drop table migration_0003_track_rating;
