create table preset (
  id uuid primary key,
  name text not null,
  base_table text not null,
  section text not null, -- 'filter' | 'sort' | 'display'
  definition text not null, -- Raw Querydown fragment for the section
  created_at timestamp_s not null,
  modified_at timestamp_s not null
);
