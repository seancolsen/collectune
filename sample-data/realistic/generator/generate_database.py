#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   # Must be the DuckDB version the backend links against, so that the file
#   # this script writes into is the same storage format the app reads back.
#   # The Rust side pins it as `duckdb = "1.10504.0"` in backend/Cargo.toml,
#   # which bundles DuckDB v1.5.4; keep the two in step.
#   "duckdb==1.5.4",
#   "pyyaml>=6.0.0",
# ]
# ///
"""
Build `radiocrate.db` for the realistic sample collection.

The work comes in two halves. First, RadioCrate's own scanner is run over the
generated FLAC files, so everything the app derives from a file — artists,
albums, tags, durations, credits, hashes — arrives exactly as it would from a
real collection. The point is to exercise the scanner rather than to
reimplement it, which also means this pipeline notices when the scanner breaks.

Second, the ratings and play logs from `collection.yaml` are loaded in. Those
are the user's own data: no file carries them, so the scanner cannot produce
them, and they can only be written straight into the database.

The result is deliberately *not* committed. It is full of freshly minted UUIDs
and scan timestamps, so no two runs produce the same bytes — and play times are
recorded in the definition relative to *now*, which makes regenerating the
database the very thing that keeps the listening history looking current.
"""

import os
import subprocess
import sys
import unicodedata
from datetime import datetime, timedelta
from pathlib import Path

import duckdb
import yaml

from layout import relative_path

# Resolved, because the scanner runs as a subprocess from the repo root:
# every path handed to it has to survive leaving this directory.
SCRIPT_DIR = Path(__file__).resolve().parent
DEFINITION = SCRIPT_DIR / "collection.yaml"
COLLECTION_DIR = SCRIPT_DIR.parent / "collection"
REPO_ROOT = SCRIPT_DIR.parents[2]
DB_PATH = COLLECTION_DIR / "radiocrate.db"

# How long before its first play an album is taken to have been added. Without
# this the scanner's `now()` would file every album as acquired seconds ago,
# under a listening history running two years back.
ACQUIRED_BEFORE_FIRST_PLAY = timedelta(days=1)


def load_definition() -> dict:
    if not DEFINITION.exists():
        sys.exit(f"Error: no collection definition at {DEFINITION}")
    return yaml.safe_load(DEFINITION.read_text())


def scanned_path(template: str, album: dict, track: dict) -> str:
    """The path the scanner will store for a track.

    `classify::normalize_path` records each file relative to the collection
    root, prefixed with `./`, which is what the import joins on.
    """
    return f"./{relative_path(template, album, track)}"


def album_acquired_at(album: dict, now: datetime) -> datetime:
    """When an album should look like it entered the library.

    Just before it was first played — the largest "days ago" among its tracks,
    since those count backwards from now.
    """
    first_play = max(
        (max(track["plays"]) for track in album["tracks"] if track["plays"]),
        default=0.0,
    )
    return now - timedelta(days=first_play) - ACQUIRED_BEFORE_FIRST_PLAY


def backdate_files(definition: dict, now: datetime) -> None:
    """Set each file's mtime to when its album was acquired.

    Done before the scan, because the scanner reads mtime off the filesystem;
    `file.added` is caught up afterwards, in the import.
    """
    missing = []
    for album in definition["albums"]:
        acquired = album_acquired_at(album, now).timestamp()
        for track in album["tracks"]:
            path = COLLECTION_DIR / relative_path(definition["path_template"], album, track)
            if not path.exists():
                missing.append(path)
                continue
            os.utime(path, (acquired, acquired))
    if missing:
        print(f"Error: {len(missing)} file(s) named in the definition are missing, e.g.")
        for path in missing[:3]:
            print(f"  {path}")
        sys.exit("Run generate_collection.py first.")


def run_scanner() -> None:
    """Scan the collection with the backend's own scanner.

    Built and run through cargo rather than hunted for under target/, so that
    the build profile and the target directory stay cargo's business. The build
    output is left visible: the first run compiles the backend, which is not
    quick, and silence there looks like a hang.
    """
    command = [
        "cargo",
        "run",
        "-p",
        "backend",
        "--",
        "scan",
        str(COLLECTION_DIR),
        "--db-path",
        str(DB_PATH),
    ]
    print(f"$ {' '.join(command)}\n")
    result = subprocess.run(command, cwd=REPO_ROOT, check=False)
    if result.returncode != 0:
        sys.exit(f"Error: the scanner exited {result.returncode}")


def stage_user_data(conn: duckdb.DuckDBPyConnection, definition: dict, now: datetime) -> None:
    """Load the definition's per-track data into two temporary tables.

    Everything is keyed by file path, and every path is NFC-normalized on both
    sides of the join: a filesystem may hand the scanner a decomposed (NFD)
    spelling of the same name — macOS does — and the typographic apostrophes and
    accents in these titles would then match nothing at all.
    """
    conn.execute("""
        create temp table imported_track (
          path text not null,
          rating float not null,
          acquired timestamp not null
        )
    """)
    conn.execute("""
        create temp table imported_play (
          path text not null,
          played_at timestamp not null
        )
    """)

    ratings = definition["ratings"]
    template = definition["path_template"]
    tracks, plays = [], []
    for album in definition["albums"]:
        acquired = album_acquired_at(album, now)
        for track in album["tracks"]:
            path = unicodedata.normalize("NFC", scanned_path(template, album, track))
            tracks.append((path, ratings[track["rating"]], acquired))
            plays.extend(
                # `play.timestamp` is second-resolution, and the definition
                # keeps every play of a track minutes apart, so truncating here
                # cannot collide two of them on the table's primary key.
                (path, (now - timedelta(days=days_ago)).replace(microsecond=0))
                for days_ago in track["plays"]
            )

    conn.executemany("insert into imported_track values (?, ?, ?)", tracks)
    conn.executemany("insert into imported_play values (?, ?)", plays)


def check_every_track_was_scanned(conn: duckdb.DuckDBPyConnection) -> None:
    """Fail loudly if the definition and the scan disagree about a path.

    A missed path would otherwise import as nothing in particular: no rating,
    no plays, and a collection that looks subtly under-used.
    """
    unmatched = conn.execute("""
        select i.path
        from imported_track i
        left join file f on nfc_normalize(f.path) = i.path
        where f.id is null
        order by i.path
    """).fetchall()
    if unmatched:
        print(f"Error: {len(unmatched)} track(s) in the definition were not scanned, e.g.")
        for (path,) in unmatched[:3]:
            print(f"  {path}")
        sys.exit("The definition and the generated files are out of step.")


def import_user_data(conn: duckdb.DuckDBPyConnection) -> None:
    """Apply the staged ratings, play logs, and acquisition dates."""
    conn.execute("""
        update track set rating = m.rating
        from (
          select t.id as track, r.id as rating
          from imported_track i
          join file f on nfc_normalize(f.path) = i.path
          join track t on t.file = f.id
          join rating r on r.value = i.rating
        ) m
        where track.id = m.track;
    """)

    # The scanner stamps every file as added at scan time; the definition knows
    # better, so the two are reconciled here rather than in the scanner.
    conn.execute("""
        update file set added = m.acquired
        from (
          select f.id as file, i.acquired as acquired
          from imported_track i
          join file f on nfc_normalize(f.path) = i.path
        ) m
        where file.id = m.file;
    """)

    conn.execute("""
        insert into play (track, "timestamp")
        select t.id, i.played_at
        from imported_play i
        join file f on nfc_normalize(f.path) = i.path
        join track t on t.file = f.id;
    """)


def summarize(conn: duckdb.DuckDBPyConnection, definition: dict) -> None:
    """Report what landed, and check it against what the definition asked for."""
    expected_tracks = sum(len(album["tracks"]) for album in definition["albums"])
    expected_plays = sum(
        len(track["plays"]) for album in definition["albums"] for track in album["tracks"]
    )

    counts = dict(
        conn.execute("""
            select 'files', count(*) from file
            union all select 'albums', count(*) from album
            union all select 'tracks', count(*) from track
            union all select 'artists', count(*) from artist
            union all select 'tags', count(*) from tag
            union all select 'rated tracks', count(*) from track where rating is not null
            union all select 'plays', count(*) from play
        """).fetchall()
    )

    print()
    for name, count in counts.items():
        print(f"  {count:>6}  {name}")

    mismatches = [
        f"{name}: {counts[name]} in the database, {expected} in the definition"
        for name, expected in (
            ("tracks", expected_tracks),
            ("rated tracks", expected_tracks),
            ("plays", expected_plays),
            ("albums", len(definition["albums"])),
        )
        if counts[name] != expected
    ]
    if mismatches:
        print()
        sys.exit("Error: " + "\n       ".join(mismatches))


def generate_database() -> None:
    definition = load_definition()
    if not COLLECTION_DIR.exists():
        sys.exit(f"Error: no collection at {COLLECTION_DIR}. Run generate_collection.py first.")

    # One instant anchors the whole run, so that "days ago" means the same thing
    # for the first track imported and the last.
    now = datetime.now().replace(microsecond=0)

    # A stale database would leave the scanner reconciling against files it
    # already knows, and the import inserting a second copy of every play.
    for path in (DB_PATH, DB_PATH.with_name(DB_PATH.name + ".wal")):
        if path.exists():
            print(f"Removing existing database at {path}")
            path.unlink()

    backdate_files(definition, now)
    run_scanner()

    print(f"\nImporting ratings and play logs into {DB_PATH}")
    with duckdb.connect(str(DB_PATH)) as conn:
        stage_user_data(conn, definition, now)
        check_every_track_was_scanned(conn)
        import_user_data(conn)
        summarize(conn, definition)

    print(f"\n✓ {DB_PATH} is ready")


if __name__ == "__main__":
    generate_database()
