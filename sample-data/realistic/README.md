# The realistic sample collection

A small, standardized music library to develop and test against, so that work
doesn't depend on whatever personal collection happens to be on the machine.

Six albums, 63 tracks, tagged with real-world metadata — titles, durations,
artist credits, genres and years, taken from MusicBrainz:

| Album | Tracks |
| --- | --- |
| The Beatles — *Sgt. Pepper's Lonely Hearts Club Band* (1967) | 13 |
| The Velvet Underground & Nico — *The Velvet Underground & Nico* (1967) | 11 |
| Pink Floyd — *The Dark Side of the Moon* (1973) | 10 |
| Fleetwood Mac — *Rumours* (1977) | 11 |
| Michael Jackson — *Thriller* (1982) | 9 |
| Prince and The Revolution — *Purple Rain* (1984) | 9 |

**The audio is silent.** Each file runs for exactly as long as the real
recording, but contains nothing — which keeps the collection free of anyone
else's copyright and small enough to commit: all 4h18m of it is under 3 MiB,
because FLAC stores a long run of identical samples in almost no space at all.

## Layout

- `generator/collection.yaml` — the source of truth. Every album, track,
  duration and tag lives here, along with the user-specific data (a rating and a
  play log per track) that belongs in the database rather than in a file's tags.
  Its header comments explain each field.
- `generator/generate_collection.py` — turns the definition into FLAC files.
- `generator/generate_database.py` — scans those files with RadioCrate's own
  scanner, then imports the ratings and play logs.
- `generator/generate_all.py` — both of the above, in order.
- `generator/layout.py` — the one rule for where a track's file goes, shared by
  the two generators so the import can find what the writer wrote.
- `collection/` — the generated files. The FLACs are committed, so using them
  needs no tooling at all; `radiocrate.db` is not (see below).

## Regenerating

All three scripts are [uv](https://docs.astral.sh/uv/) single-file scripts: each
declares its own dependencies and Python version inline, and uv fetches both on
first run. With [uv installed](https://docs.astral.sh/uv/getting-started/installation/)
— the dev container has it — nothing else needs setting up. Run one directly, or
equivalently as `uv run <script>`:

```sh
./sample-data/realistic/generator/generate_all.py
```

### Just the audio files

```sh
./sample-data/realistic/generator/generate_collection.py
```

The files are checked in, so this is only needed after editing `collection.yaml`
— adding a track, fixing a duration, correcting a tag. It rebuilds `collection/`
from scratch, so a track dropped or renamed in the definition leaves no stale
file behind, and it takes well under a minute. Commit the result alongside the
change to `collection.yaml`.

Ratings and play logs are deliberately *not* written into the files. They aren't
file metadata; they're the user's own data, and only the database holds them.

### Just the database

```sh
./sample-data/realistic/generator/generate_database.py
```

This builds `collection/radiocrate.db` in two steps. First it runs the real
scanner — `cargo run -p backend -- scan …`, so the first run compiles the
backend — which means everything the app derives from a file arrives exactly as
it would from a real collection, and a broken scanner shows up here. Then it
opens the fresh database and imports what no file can carry: each track's rating
and play log, joined to the scanned rows on file path.

The database is **not committed**, and is gitignored. It is full of freshly
minted UUIDs and scan timestamps, so no two runs produce the same bytes — and
the definition stores play times as *days before generation*, which makes
re-running this the very thing that keeps the listening history looking current.
Rebuild it whenever the history has drifted, or after a schema migration.

The import fails loudly rather than quietly under-filling the database: if a
track in the definition doesn't match a scanned file, or the final counts don't
match what the definition asked for, it says so and exits non-zero.

## Using the collection

Once the database exists, point the server at the collection and skip the
startup scan:

```sh
cargo run -p backend -- serve sample-data/realistic/collection --no-scan
```

Without `--db-path`, the server uses `collection/radiocrate.db` — exactly what
the generator built.
