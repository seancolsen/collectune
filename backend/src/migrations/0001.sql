create type format as enum (
  'aac',
  'adpcm',
  'aiff',
  'alac',
  'ape',
  'caf',
  'flac',
  'mkv',
  'mp1',
  'mp2',
  'mp3',
  'mp4',
  'ogg',
  'opus',
  'vorbis',
  'wav',
  'webm',
  'wma',
  'wv'
);

create table deletion (
  id uuid primary key,
  timestamp datetime not null default now(),
  comment text
);

create table file (
  id uuid primary key,
  path text unique not null,
  hash blob not null, -- (Note: possible for two files to have the same hash)
  size uinteger not null,
  format format not null,
  duration real not null,
  mtime bigint not null, -- filesystem mtime as microseconds since epoch
  added timestamp not null,
  deletion uuid
);

create table artist (
  id uuid primary key,
  name text unique not null
);

create table album (
  id uuid primary key,
  title text,
  year usmallint
);

create table track (
  id uuid primary key,
  file uuid not null,
  start_position real,
  end_position real,
  title text,
  album uuid,
  disc_number utinyint,
  track_number utinyint,
  genre text,
  rating real
);

create table credit (
  track uuid not null,
  artist uuid not null,
  ord real,
  role text,
  primary key (track, artist)
);

