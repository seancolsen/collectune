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
- `collection/` — the generated files, committed so that using them needs no
  tooling at all.

## Regenerating the files

The generated files are checked in, so this is only needed after editing
`collection.yaml` — adding a track, fixing a duration, correcting a tag.

The script is a [uv](https://docs.astral.sh/uv/) single-file script: it declares
its own dependencies and Python version inline, and uv fetches both on first
run. With [uv installed](https://docs.astral.sh/uv/getting-started/installation/),
nothing else needs setting up:

```sh
./sample-data/realistic/generator/generate_collection.py
```

or, equivalently:

```sh
uv run sample-data/realistic/generator/generate_collection.py
```

It rebuilds `collection/` from scratch — a track dropped or renamed in the
definition leaves no stale file behind — and takes well under a minute. Commit
the result alongside the change to `collection.yaml`.

Note that ratings and play logs are deliberately *not* written into the files.
They aren't file metadata; they're the user's own data, and they're loaded into
the database separately.

## Using the collection

Point the server at it like any other music directory:

```sh
cargo run -p backend -- sample-data/realistic/collection --db-path /tmp/sample.db
```

`--db-path` is worth passing: without it the database is created as
`radiocrate.db` *inside* the collection directory, in the middle of the
committed files. (It's gitignored either way.)
