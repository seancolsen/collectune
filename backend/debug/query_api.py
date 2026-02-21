#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "pyarrow",
#     "urllib3",
# ]
# ///

import pyarrow.ipc as ipc
import urllib3

QUERY = """
with
  a as (select id from album where title ilike '%kpop%'),
  c as (
    select
      credit.track,
      array_agg(artist.name) as artists
    from credit
    join artist on artist.id = credit.artist
    group by credit.track
  )
select
  track.title,
  c.artists
from track
join a on a.id = track.album
left join c on c.track = track.id
"""

URL = "http://localhost:3000/query"

resp = urllib3.request("POST", URL, body=QUERY.encode(), preload_content=False)

if resp.status != 200:
    print(f"Error {resp.status}: {resp.data.decode()}")
    raise SystemExit(1)

reader = ipc.open_stream(resp.read())
print(f"Schema: {reader.schema}\n")
for batch in reader:
    columns = batch.to_pydict()
    names = batch.schema.names
    rows = zip(*(columns[n] for n in names))
    widths = [max(len(n), max((len(str(v)) for v in columns[n]), default=0)) for n in names]
    print("  ".join(n.ljust(w) for n, w in zip(names, widths)))
    print("  ".join("-" * w for w in widths))
    for row in rows:
        print("  ".join(str(v).ljust(w) for v, w in zip(row, widths)))
